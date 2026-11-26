use std::path::{Path, PathBuf};

use crate::linux::convert::pkgbuild;
use crate::linux::distro::{which, SystemInfo};
use crate::linux::elevate::Elevator;
use crate::linux::job::{JobContext, JobError, JobResult};
use crate::linux::manage::Sink;
use crate::linux::model::{DepStatus, InstallKind, InstalledEntry, JobEvent, StageId};
use crate::linux::proc::run_streamed;
use crate::linux::registry::now_rfc3339;

use super::shortcuts::ensure_shortcuts;

pub(crate) const PACMAN_LOCK: &str = "/var/lib/pacman/db.lck";
pub(crate) const PACMAN_WAIT: std::time::Duration = std::time::Duration::from_secs(300);

pub(crate) async fn wait_for_pacman(
    lock: &Path,
    limit: std::time::Duration,
    say: impl Fn(&str),
    cancelled: impl Fn() -> bool,
) -> JobResult<()> {
    if !lock.exists() {
        return Ok(());
    }
    say("pacman is busy with another program, waiting for it to finish");
    let start = std::time::Instant::now();
    while lock.exists() {
        if cancelled() {
            return Err(JobError::cancelled());
        }
        if start.elapsed() >= limit {
            return Err(JobError {
                message: "pacman stayed busy with another program for too long.".into(),
                hint: Some(format!(
                    "Let the other update or install finish, then try again. If nothing is using pacman, a crash left {} behind: remove it with sudo rm {}.",
                    lock.display(),
                    lock.display()
                )),
            });
        }
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    }
    Ok(())
}

fn stage_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    if std::fs::hard_link(src, dest).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dest).map(|_| ())
}

fn find_built_package(dir: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().contains(".pkg.tar"))
                .unwrap_or(false)
        })
        .collect();
    found.sort();
    found.pop()
}

fn hint_for(tail: &str) -> Option<String> {
    let t = tail.to_lowercase();
    if t.contains("conflicting files") || t.contains("exists in filesystem") {
        Some("Another package already owns some of these files. Remove that package first, or install the version from the Arch repositories instead.".into())
    } else if t.contains("could not satisfy dependencies") || t.contains("unable to satisfy") {
        Some("A translated dependency isn't available in your repositories. Check the dependency list on the sheet and the raw log.".into())
    } else if t.contains("unable to lock database") || t.contains("db.lck") {
        Some("Another package manager is running (or a stale /var/lib/pacman/db.lck is left over). Wait for it to finish and try again.".into())
    } else if t.contains("fakeroot") {
        Some("makepkg needs fakeroot. Install base-devel from Settings → Tools.".into())
    } else {
        None
    }
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
    let makepkg = which("makepkg").ok_or_else(|| {
        JobError::with_hint(
            "makepkg isn't installed.",
            "Install base-devel from Settings → Tools.",
        )
    })?;
    let pacman = which("pacman").ok_or_else(|| JobError::new("pacman isn't installed?!"))?;

    ctx.begin(StageId::Inspect);
    ctx.narrate(format!("Checking {}…", model.display_name));
    ctx.finish(StageId::Inspect);

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
    let unresolved: Vec<String> = ctx
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
    if total == 0 {
        ctx.narrate("No dependencies to translate");
    } else {
        ctx.narrate(format!(
            "Translated {} of {} dependencies to Arch package names",
            depends.len(),
            total
        ));
    }
    if !unresolved.is_empty() {
        ctx.narrate(format!("Going ahead without: {}", unresolved.join(", ")));
    }
    ctx.check_cancelled()?;
    ctx.finish(StageId::Translate);

    ctx.begin(StageId::Build);
    let pkgname = model.name.clone();
    let mut build = crate::linux::convert::BuildDir::create(&pkgname)?;
    let dir = build.path().to_path_buf();
    let source_name = format!(
        "source.{}",
        model.kind.label().trim_start_matches('.').to_lowercase()
    );
    stage_file(&src, &dir.join(&source_name)).map_err(|e| {
        JobError::new(format!(
            "Couldn't copy the package into the build folder: {e}"
        ))
    })?;
    let pkgbuild = pkgbuild::generate(model, &pkgname, &depends, &source_name);
    std::fs::write(dir.join("PKGBUILD"), &pkgbuild)?;
    for line in pkgbuild.lines() {
        ctx.log(&format!("PKGBUILD| {line}"));
    }
    ctx.narrate(format!(
        "Building an Arch package for {} (this is the slow bit)",
        model.display_name
    ));
    let dir_s = dir.to_string_lossy().into_owned();
    let env = [
        ("PKGDEST", dir_s.as_str()),
        ("SRCDEST", dir_s.as_str()),
        ("BUILDDIR", dir_s.as_str()),
        ("LOGDEST", dir_s.as_str()),
        ("PKGEXT", ".pkg.tar"),
        ("PACKAGER", "ZLauncher <zlauncher@localhost>"),
    ];
    let out = run_streamed(
        ctx,
        &makepkg,
        &[
            "-f",
            "--noconfirm",
            "--nodeps",
            "--skipinteg",
            "--noprogressbar",
        ],
        Some(&dir),
        &env,
    )
    .await?;
    ctx.check_cancelled()?;
    if !out.success {
        ctx.fail(StageId::Build);
        build.keep();
        return Err(JobError {
            message: format!(
                "makepkg failed (exit {}). The build folder was kept at {}.",
                out.code.unwrap_or(-1),
                dir.display()
            ),
            hint: hint_for(&out.tail_text()),
        });
    }
    let built = find_built_package(&dir).ok_or_else(|| {
        JobError::new(format!(
            "makepkg finished but no package appeared in {}",
            dir.display()
        ))
    })?;
    ctx.log(&format!("built {}", built.display()));
    ctx.finish(StageId::Build);

    ctx.begin(StageId::Install);
    let elevator = Elevator::for_context(ctx)?;
    ctx.narrate(elevator.describe("install with pacman"));
    let built_s = built.to_string_lossy().into_owned();
    wait_for_pacman(Path::new(PACMAN_LOCK), PACMAN_WAIT, |t| ctx.narrate(t), || ctx.is_cancelled()).await?;
    let out = elevator
        .run(ctx, &pacman, &["-U", "--noconfirm", &built_s])
        .await?;
    if !out.success {
        ctx.fail(StageId::Install);
        return Err(JobError {
            message: format!(
                "pacman refused the package (exit {}).",
                out.code.unwrap_or(-1)
            ),
            hint: hint_for(&out.tail_text()),
        });
    }
    ctx.narrate(format!(
        "{} is installed — pacman tracks it as {pkgname}",
        model.display_name
    ));
    drop(build);
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
        host_package: Some(pkgname),
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
    let pacman = which("pacman").ok_or_else(|| JobError::new("pacman isn't installed?!"))?;
    let _ = sink.send(JobEvent::Narrate {
        text: format!("Removing {pkg} with administrator rights"),
    });
    let installed = std::process::Command::new(&pacman)
        .args(["-Qq", "--", &pkg])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !installed {
        let _ = sink.send(JobEvent::Narrate {
            text: format!("{pkg} was already gone from pacman"),
        });
        return Ok(());
    }
    wait_for_pacman(
        Path::new(PACMAN_LOCK),
        PACMAN_WAIT,
        |t| {
            let _ = sink.send(JobEvent::Narrate { text: t.into() });
        },
        || false,
    )
    .await?;
    let out = super::run_root_simple(system, sink, &pacman, &["-R", "--noconfirm", &pkg]).await?;
    if !out.success {
        return Err(JobError {
            message: format!(
                "pacman couldn't remove {pkg} (exit {}).",
                out.code.unwrap_or(-1)
            ),
            hint: hint_for(&out.tail_text()),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::time::Duration;

    #[tokio::test]
    async fn no_lock_goes_straight_on() {
        let dir = tempfile::tempdir().unwrap();
        let said = Cell::new(0);
        let r = wait_for_pacman(&dir.path().join("db.lck"), Duration::from_secs(5), |_| said.set(said.get() + 1), || false).await;
        assert!(r.is_ok());
        assert_eq!(said.get(), 0);
    }

    #[tokio::test]
    async fn waits_until_the_lock_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("db.lck");
        std::fs::write(&lock, b"").unwrap();
        let gone = lock.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(700)).await;
            std::fs::remove_file(gone).unwrap();
        });
        let said = Cell::new(0);
        let start = std::time::Instant::now();
        let r = wait_for_pacman(&lock, Duration::from_secs(10), |_| said.set(said.get() + 1), || false).await;
        assert!(r.is_ok());
        assert_eq!(said.get(), 1);
        assert!(start.elapsed() >= Duration::from_millis(600));
    }

    #[tokio::test]
    async fn gives_up_with_a_hint() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("db.lck");
        std::fs::write(&lock, b"").unwrap();
        let e = wait_for_pacman(&lock, Duration::from_millis(300), |_| {}, || false).await.unwrap_err();
        assert!(e.message.contains("too long"));
        assert!(e.hint.unwrap().contains("db.lck"));
    }

    #[tokio::test]
    async fn stops_when_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("db.lck");
        std::fs::write(&lock, b"").unwrap();
        let e = wait_for_pacman(&lock, Duration::from_secs(10), |_| {}, || true).await.unwrap_err();
        assert_eq!(e.message, "Cancelled.");
    }
}
