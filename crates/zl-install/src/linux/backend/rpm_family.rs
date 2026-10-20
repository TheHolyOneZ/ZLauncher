use std::path::PathBuf;

use crate::linux::convert::{extract, rpmspec};
use crate::linux::distro::{which, Family, SystemInfo};
use crate::linux::elevate::Elevator;
use crate::linux::job::{JobContext, JobError, JobResult};
use crate::linux::manage::Sink;
use crate::linux::model::{DepStatus, InstallKind, InstalledEntry, JobEvent, PackageKind, StageId};
use crate::linux::proc::run_streamed;
use crate::linux::registry::now_rfc3339;

use super::shortcuts::ensure_shortcuts;

fn hint_for(tail: &str) -> Option<String> {
    let t = tail.to_lowercase();
    if t.contains("nothing provides")
        || t.contains("no match for")
        || t.contains("not found in package names")
    {
        Some("A dependency isn't available from your repositories. Check the dependency list on the sheet and the raw log.".into())
    } else if t.contains("conflicts with file") || t.contains("file from install of") {
        Some("Another package already owns some of these files. Remove that package first.".into())
    } else if t.contains("lock") && (t.contains("held") || t.contains("waiting")) {
        Some("Another package manager is running. Wait for it to finish and try again.".into())
    } else if t.contains("gpg") || t.contains("signature") {
        Some("The package isn't signed with a key your system trusts. ZLauncher passes --nogpgcheck for local files, so this may be a repository key problem instead.".into())
    } else {
        None
    }
}

struct Pm {
    bin: PathBuf,
    install: Vec<String>,
    remove: Vec<String>,
    label: &'static str,
}

fn pm_for(family: Family) -> JobResult<Pm> {
    match family {
        Family::Suse => {
            let bin = which("zypper").ok_or_else(|| JobError::new("zypper isn't installed?!"))?;
            Ok(Pm {
                bin,
                install: ["--non-interactive", "install", "--allow-unsigned-rpm"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                remove: ["--non-interactive", "remove"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                label: "zypper",
            })
        }
        _ => {
            let bin = which("dnf")
                .or_else(|| which("dnf5"))
                .or_else(|| which("yum"))
                .ok_or_else(|| JobError::new("dnf isn't installed?!"))?;
            Ok(Pm {
                bin,
                install: ["install", "-y", "--nogpgcheck"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                remove: ["remove", "-y"].iter().map(|s| s.to_string()).collect(),
                label: "dnf",
            })
        }
    }
}

fn find_built_rpm(topdir: &std::path::Path) -> Option<PathBuf> {
    let rpms = topdir.join("RPMS");
    let mut found = Vec::new();
    for arch_dir in std::fs::read_dir(&rpms).ok()?.flatten() {
        for f in std::fs::read_dir(arch_dir.path()).ok()?.flatten() {
            if f.path().extension().map(|e| e == "rpm").unwrap_or(false) {
                found.push(f.path());
            }
        }
    }
    found.sort();
    found.pop()
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
    let pm = pm_for(ctx.system.family)?;

    ctx.begin(StageId::Inspect);
    ctx.narrate(format!("Checking {}…", model.display_name));
    ctx.finish(StageId::Inspect);

    let mut build: Option<crate::linux::convert::BuildDir> = None;
    let mut unresolved: Vec<String> = Vec::new();

    let rpm_to_install: PathBuf = match model.kind {
        PackageKind::Rpm => src.clone(),
        PackageKind::Deb => {
            ctx.begin(StageId::Translate);
            let requires: Vec<String> = {
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
                    "Translated {} of {total} dependencies for {}",
                    requires.len(),
                    ctx.system.family.label()
                )
            });
            if !unresolved.is_empty() {
                ctx.narrate(format!("Going ahead without: {}", unresolved.join(", ")));
            }
            ctx.check_cancelled()?;
            ctx.finish(StageId::Translate);

            ctx.begin(StageId::Build);
            let rpmbuild = which("rpmbuild").ok_or_else(|| {
                JobError::with_hint(
                    "rpmbuild isn't installed.",
                    "Install rpm-build from Settings → Tools.",
                )
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
            let tree = extract::list_tree(&root)?;
            let files_list = dir.join("files.list");
            std::fs::write(&files_list, rpmspec::files_list(&tree))?;
            let topdir = dir.join("rpm");
            for sub in ["BUILD", "BUILDROOT", "RPMS", "SOURCES", "SPECS", "SRPMS"] {
                std::fs::create_dir_all(topdir.join(sub))?;
            }
            let spec_text = rpmspec::spec(
                model,
                &model.name,
                &requires,
                &root.to_string_lossy(),
                &files_list.to_string_lossy(),
            );
            let spec_path = topdir.join("SPECS").join(format!("{}.spec", model.name));
            std::fs::write(&spec_path, &spec_text)?;
            for line in spec_text.lines() {
                ctx.log(&format!("spec| {line}"));
            }
            ctx.narrate(format!(
                "Building an RPM for {} (this is the slow bit)",
                model.display_name
            ));
            let topdir_def = format!("_topdir {}", topdir.display());
            let out = run_streamed(
                ctx,
                &rpmbuild,
                &[
                    "-bb",
                    "--define",
                    &topdir_def,
                    "--define",
                    "_binary_payload w.ufdio",
                    "--define",
                    "_build_id_links none",
                    "--nodeps",
                    &spec_path.to_string_lossy(),
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
                return Err(JobError {
                    message: format!(
                        "rpmbuild failed (exit {}). The build folder was kept at {}.",
                        out.code.unwrap_or(-1),
                        dir.display()
                    ),
                    hint: hint_for(&out.tail_text()),
                });
            }
            let built = find_built_rpm(&topdir).ok_or_else(|| {
                JobError::new(format!(
                    "rpmbuild finished but no .rpm appeared under {}",
                    topdir.display()
                ))
            })?;
            ctx.log(&format!("built {}", built.display()));
            ctx.finish(StageId::Build);
            built
        }
        PackageKind::AppImage => unreachable!("AppImages never reach the RPM backend"),
    };

    ctx.begin(StageId::Install);
    let elevator = Elevator::for_context(ctx)?;
    ctx.narrate(elevator.describe(&format!("install with {}", pm.label)));
    let abs = std::fs::canonicalize(&rpm_to_install).unwrap_or(rpm_to_install.clone());
    let abs_s = abs.to_string_lossy().into_owned();
    let mut args: Vec<&str> = pm.install.iter().map(String::as_str).collect();
    args.push(&abs_s);
    let out = elevator.run(ctx, &pm.bin, &args).await?;
    if !out.success {
        ctx.fail(StageId::Install);
        return Err(JobError {
            message: format!(
                "{} refused the package (exit {}).",
                pm.label,
                out.code.unwrap_or(-1)
            ),
            hint: hint_for(&out.tail_text()),
        });
    }

    let host_package = std::process::Command::new("rpm")
        .args(["-qp", "--qf", "%{NAME}", "--", &abs_s])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| model.name.clone());
    ctx.narrate(format!(
        "{} is installed — {} tracks it as {host_package}",
        model.display_name, pm.label
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
        host_package: Some(host_package),
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
    let pm = pm_for(system.family)?;
    let installed = std::process::Command::new("rpm")
        .args(["-q", "--", &pkg])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !installed {
        let _ = sink.send(JobEvent::Narrate {
            text: format!("{pkg} was already gone from {}", pm.label),
        });
        return Ok(());
    }
    let _ = sink.send(JobEvent::Narrate {
        text: format!("Removing {pkg} with administrator rights"),
    });
    let mut args: Vec<&str> = pm.remove.iter().map(String::as_str).collect();
    args.push(&pkg);
    let out = super::run_root_simple(system, sink, &pm.bin, &args).await?;
    if !out.success {
        return Err(JobError {
            message: format!(
                "{} couldn't remove {pkg} (exit {}).",
                pm.label,
                out.code.unwrap_or(-1)
            ),
            hint: hint_for(&out.tail_text()),
        });
    }
    Ok(())
}
