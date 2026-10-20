use std::path::PathBuf;

use crate::linux::convert::{debbuild, extract};
use crate::linux::distro::{which, SystemInfo};
use crate::linux::elevate::Elevator;
use crate::linux::job::{JobContext, JobError, JobResult};
use crate::linux::manage::Sink;
use crate::linux::model::{DepStatus, InstallKind, InstalledEntry, JobEvent, PackageKind, StageId};
use crate::linux::proc::run_streamed;
use crate::linux::registry::now_rfc3339;

use super::shortcuts::ensure_shortcuts;

fn hint_for(tail: &str) -> Option<String> {
    let t = tail.to_lowercase();
    if t.contains("unmet dependencies") || t.contains("depends:") && t.contains("not installable") {
        Some("A dependency isn't available from your apt sources. Run `sudo apt update` and try again, or check the dependency list on the sheet.".into())
    } else if t.contains("could not get lock") || t.contains("lock-frontend") {
        Some("Another package manager (apt, Software Center, unattended-upgrades) is running. Wait for it to finish and try again.".into())
    } else if t.contains("trying to overwrite") {
        Some("Another package already owns some of these files. Remove that package first.".into())
    } else {
        None
    }
}

async fn apt_root(
    ctx: &JobContext,
    elevator: &Elevator,
    args: &[&str],
) -> JobResult<crate::linux::proc::Output> {
    let env = which("env").unwrap_or_else(|| PathBuf::from("/usr/bin/env"));
    let apt = which("apt-get").ok_or_else(|| JobError::new("apt-get isn't installed?!"))?;
    let apt_s = apt.to_string_lossy().into_owned();
    let mut full: Vec<&str> = vec!["DEBIAN_FRONTEND=noninteractive", apt_s.as_str()];
    full.extend_from_slice(args);
    elevator.run(ctx, &env, &full).await
}

pub async fn install(ctx: &JobContext) -> JobResult<InstalledEntry> {
    let model = &ctx.plan.package;
    let src = model.source_path.clone();
    if !src.exists() {
        return Err(JobError::new(format!(
            "{} is gone — was it moved or deleted?",
            src.display()
        )));
    }

    ctx.begin(StageId::Inspect);
    ctx.narrate(format!("Checking {}…", model.display_name));
    ctx.finish(StageId::Inspect);

    let mut build: Option<crate::linux::convert::BuildDir> = None;
    let mut unresolved: Vec<String> = Vec::new();

    let deb_to_install: PathBuf = match model.kind {
        PackageKind::Deb => src.clone(),
        PackageKind::Rpm => {
            ctx.begin(StageId::Translate);
            let depends: Vec<String> = {
                let mut v: Vec<String> = ctx
                    .plan
                    .dependencies
                    .iter()
                    .filter(|d| matches!(d.status, DepStatus::Found))
                    .filter_map(|d| d.resolved.clone())
                    .collect();
                v.sort();
                v.dedup();
                v
            };
            unresolved = ctx
                .plan
                .dependencies
                .iter()
                .filter(|d| matches!(d.status, DepStatus::Unresolved))
                .map(|d| d.raw.clone())
                .collect();
            let total = ctx
                .plan
                .dependencies
                .iter()
                .filter(|d| !matches!(d.status, DepStatus::Skipped { .. }))
                .count();
            ctx.narrate(if total == 0 {
                "No dependencies to translate".to_string()
            } else {
                format!(
                    "Translated {} of {total} dependencies to Debian package names",
                    depends.len()
                )
            });
            if !unresolved.is_empty() {
                ctx.narrate(format!("Going ahead without: {}", unresolved.join(", ")));
            }
            ctx.check_cancelled()?;
            ctx.finish(StageId::Translate);

            ctx.begin(StageId::Build);
            let dpkg_deb = which("dpkg-deb").ok_or_else(|| {
                JobError::with_hint("dpkg-deb isn't installed.", "Install the dpkg package.")
            })?;
            let guard = crate::linux::convert::BuildDir::create(&model.name)?;
            let dir = guard.path().to_path_buf();
            build = Some(guard);
            let root = dir.join("root");
            ctx.narrate(format!("Unpacking {}", model.display_name));
            let (k, s, r) = (model.kind, src.clone(), root.clone());
            tokio::task::spawn_blocking(move || extract::extract_tree(k, &s, &r))
                .await
                .map_err(|e| JobError::new(e.to_string()))?
                .map_err(|e| JobError::new(format!("Couldn't unpack the package: {e}")))?;
            let size = extract::tree_size(&root);
            std::fs::create_dir_all(root.join("DEBIAN"))?;
            let control = debbuild::control(model, &model.name, &depends, size);
            std::fs::write(root.join("DEBIAN/control"), &control)?;
            for line in control.lines() {
                ctx.log(&format!("control| {line}"));
            }
            ctx.narrate(format!(
                "Building a Debian package for {}",
                model.display_name
            ));
            let out_deb = dir.join(format!("{}.deb", model.name));
            let out = run_streamed(
                ctx,
                &dpkg_deb,
                &[
                    "--build",
                    "--root-owner-group",
                    "-Znone",
                    &root.to_string_lossy(),
                    &out_deb.to_string_lossy(),
                ],
                Some(&dir),
                &[],
            )
            .await?;
            ctx.check_cancelled()?;
            if !out.success {
                ctx.fail(StageId::Build);
                if let Some(b) = build.as_mut() {
                    b.keep();
                }
                return Err(JobError::new(format!(
                    "dpkg-deb failed (exit {}). The build folder was kept at {}.",
                    out.code.unwrap_or(-1),
                    dir.display()
                )));
            }
            ctx.finish(StageId::Build);
            out_deb
        }
        PackageKind::AppImage => unreachable!("AppImages never reach the Debian backend"),
    };

    ctx.begin(StageId::Install);
    let elevator = Elevator::for_context(ctx)?;
    ctx.narrate(elevator.describe("install with apt"));
    let abs = std::fs::canonicalize(&deb_to_install).unwrap_or(deb_to_install.clone());
    let abs_s = abs.to_string_lossy().into_owned();
    let out = apt_root(ctx, &elevator, &["install", "-y", "--", &abs_s]).await?;
    if !out.success {
        ctx.fail(StageId::Install);
        return Err(JobError {
            message: format!("apt refused the package (exit {}).", out.code.unwrap_or(-1)),
            hint: hint_for(&out.tail_text()),
        });
    }
    ctx.narrate(format!(
        "{} is installed — apt tracks it as {}",
        model.display_name, model.name
    ));
    drop(build.take());
    ctx.finish(StageId::Install);

    ctx.begin(StageId::Shortcuts);
    let shortcuts = ensure_shortcuts(ctx).await?;
    if ctx.options.remove_original {
        match std::fs::remove_file(&src) {
            Ok(()) => ctx.narrate("Deleted the installer file"),
            Err(e) => ctx.log(&format!("could not delete {}: {e}", src.display())),
        }
    }
    ctx.finish(StageId::Shortcuts);

    Ok(InstalledEntry {
        id: uuid::Uuid::new_v4().to_string(),
        name: model.display_name.clone(),
        slug: model.name.clone(),
        version: model.version.clone(),
        source_kind: model.kind,
        install_kind: InstallKind::Native,
        host_package: Some(model.name.clone()),
        appimage_path: None,
        desktop_files: shortcuts.desktop_files,
        launcher: shortcuts.launcher,
        icon_path: shortcuts.icon_path,
        original_path: src,
        installed_at: now_rfc3339(),
        unresolved_deps: unresolved,
        summary: None,
        description: None,
        maintainer: None,
        homepage: None,
        license: None,
        arch: None,
        file_size: None,
        update_dir: None,
        ignored_versions: Vec::new(),
        history: Vec::new(),
    }
    .with_metadata(model))
}

pub async fn uninstall(entry: &InstalledEntry, system: &SystemInfo, sink: &Sink) -> JobResult<()> {
    let pkg = entry
        .host_package
        .clone()
        .ok_or_else(|| JobError::new("No package name recorded for this entry."))?;
    let installed = std::process::Command::new("dpkg-query")
        .args(["-W", "-f=${Status}", "--", &pkg])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("install ok"))
        .unwrap_or(false);
    if !installed {
        let _ = sink.send(JobEvent::Narrate {
            text: format!("{pkg} was already gone from apt"),
        });
        return Ok(());
    }
    let _ = sink.send(JobEvent::Narrate {
        text: format!("Removing {pkg} with administrator rights"),
    });
    let env = which("env").unwrap_or_else(|| PathBuf::from("/usr/bin/env"));
    let apt = which("apt-get").ok_or_else(|| JobError::new("apt-get isn't installed?!"))?;
    let apt_s = apt.to_string_lossy().into_owned();
    let out = super::run_root_simple(
        system,
        sink,
        &env,
        &[
            "DEBIAN_FRONTEND=noninteractive",
            &apt_s,
            "remove",
            "-y",
            "--",
            &pkg,
        ],
    )
    .await?;
    if !out.success {
        return Err(JobError {
            message: format!(
                "apt couldn't remove {pkg} (exit {}).",
                out.code.unwrap_or(-1)
            ),
            hint: hint_for(&out.tail_text()),
        });
    }
    Ok(())
}
