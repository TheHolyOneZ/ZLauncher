use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde_json::json;
use zl_crawler::Kind;

use crate::{step, Error, Events, Installed, Request, Result};

fn applications_dir() -> PathBuf {
    let system = PathBuf::from("/Applications");
    let probe = system.join(".zlauncher-write-test");
    if std::fs::write(&probe, b"").is_ok() {
        let _ = std::fs::remove_file(&probe);
        return system;
    }
    dirs::home_dir().unwrap_or_default().join("Applications")
}

fn scratch(kind: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    std::env::temp_dir().join(format!("zlauncher-{kind}-{}-{nanos}", std::process::id()))
}

fn run(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| Error::new(format!("Couldn't start {program}: {e}")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(Error::new(format!("{program} failed: {}", String::from_utf8_lossy(&out.stderr).trim())))
    }
}

fn find_app(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "app") && p.is_dir())
}

fn app_version(app: &Path) -> Option<String> {
    let plist = app.join("Contents/Info.plist");
    run("/usr/bin/plutil", &["-extract", "CFBundleShortVersionString", "raw", "-o", "-", &plist.to_string_lossy()])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn copy_app(src: &Path, dest_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dest_dir)?;
    let dest = dest_dir.join(src.file_name().ok_or_else(|| Error::new("Bad app name."))?);
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    run("/usr/bin/ditto", &[&src.to_string_lossy(), &dest.to_string_lossy()])?;
    Ok(dest)
}

pub async fn install(req: Request<'_>, events: Events, _cancel: Arc<AtomicBool>) -> Result<Installed> {
    let file = req.file.to_path_buf();
    let kind = req.artifact.kind;
    let version = req.artifact.version.clone();
    let desktop = req.options.desktop_shortcut;
    tokio::task::spawn_blocking(move || -> Result<Installed> {
        let target = applications_dir();
        let app = if kind == Kind::Dmg {
            step(&events, "Opening the disk image");
            let mount = scratch("dmg");
            std::fs::create_dir_all(&mount)?;
            let m = mount.to_string_lossy().to_string();
            run("/usr/bin/hdiutil", &["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint", &m, &file.to_string_lossy()])?;
            let result = (|| {
                let src = find_app(&mount).ok_or_else(|| Error::new("The disk image contains no app."))?;
                step(&events, format!("Copying to {}", target.display()));
                copy_app(&src, &target)
            })();
            let _ = run("/usr/bin/hdiutil", &["detach", "-force", &m]);
            let _ = std::fs::remove_dir(&mount);
            result?
        } else {
            step(&events, "Unpacking the app");
            let tmp = scratch("app");
            std::fs::create_dir_all(&tmp)?;
            run("/usr/bin/tar", &["-xzf", &file.to_string_lossy(), "-C", &tmp.to_string_lossy()])?;
            let src = find_app(&tmp).ok_or_else(|| Error::new("The archive contains no app."))?;
            step(&events, format!("Copying to {}", target.display()));
            let app = copy_app(&src, &target);
            let _ = std::fs::remove_dir_all(&tmp);
            app?
        };
        let mut handle = json!({ "app": app });
        if desktop {
            handle = set_desktop_shortcut("app", &handle, true)?;
        }
        Ok(Installed { method: "app".into(), version: app_version(&app).or(version), handle })
    })
    .await
    .map_err(|e| Error::new(e.to_string()))?
}

fn app_of(handle: &serde_json::Value) -> Result<PathBuf> {
    handle
        .get("app")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .ok_or_else(|| Error::new("This install record has no app path."))
}

fn desktop_link(app: &Path) -> Option<PathBuf> {
    Some(dirs::desktop_dir()?.join(app.file_name()?))
}

pub async fn uninstall(_method: &str, handle: &serde_json::Value, events: Events) -> Result<()> {
    let app = app_of(handle)?;
    step(&events, format!("Moving {} to the Trash", app.display()));
    if let Some(l) = desktop_link(&app) {
        if l.is_symlink() {
            let _ = std::fs::remove_file(l);
        }
    }
    if !app.exists() {
        return Ok(());
    }
    let trash = dirs::home_dir().unwrap_or_default().join(".Trash");
    let name = app.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    let mut dest = trash.join(&name);
    if dest.exists() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        dest = trash.join(format!("{} {stamp}", name.to_string_lossy()));
    }
    if std::fs::rename(&app, &dest).is_err() {
        std::fs::remove_dir_all(&app)?;
    }
    Ok(())
}

pub fn launch(_method: &str, handle: &serde_json::Value) -> Result<()> {
    let app = app_of(handle)?;
    Command::new("/usr/bin/open").arg(&app).spawn().map(|_| ()).map_err(|e| Error::new(e.to_string()))
}

pub fn is_present(_method: &str, handle: &serde_json::Value) -> bool {
    app_of(handle).is_ok_and(|a| a.exists())
}

pub fn has_desktop_shortcut(_method: &str, handle: &serde_json::Value) -> bool {
    app_of(handle).ok().and_then(|a| desktop_link(&a)).is_some_and(|l| l.is_symlink())
}

pub fn set_desktop_shortcut(_method: &str, handle: &serde_json::Value, want: bool) -> Result<serde_json::Value> {
    let app = app_of(handle)?;
    let link = desktop_link(&app).ok_or_else(|| Error::new("There is no Desktop folder."))?;
    if want {
        if !link.exists() {
            std::os::unix::fs::symlink(&app, &link)?;
        }
    } else if link.is_symlink() {
        std::fs::remove_file(&link)?;
    }
    Ok(handle.clone())
}
