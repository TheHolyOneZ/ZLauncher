use std::path::Path;

use crate::linux::desktop;
use crate::linux::distro::{which, SystemInfo};
use crate::linux::job::{JobError, JobResult};
use crate::linux::model::{InstallKind, InstalledEntry, JobEvent};
use crate::linux::registry::Registry;

pub type Sink = tokio::sync::mpsc::UnboundedSender<JobEvent>;

fn narrate(sink: &Sink, text: impl Into<String>) {
    let _ = sink.send(JobEvent::Narrate { text: text.into() });
}

pub async fn uninstall(id: &str, system: &SystemInfo, sink: &Sink) -> JobResult<InstalledEntry> {
    let registry = Registry::load();
    let entry = registry
        .find(id)
        .cloned()
        .ok_or_else(|| JobError::new("That entry is no longer in the library."))?;

    match entry.install_kind {
        InstallKind::AppImage => {
            narrate(sink, format!("Removing {} and its shortcuts", entry.name));
            crate::linux::appimage::remove(&entry)
                .map_err(|e| JobError::new(format!("Couldn't remove files: {e}")))?;
        }
        InstallKind::Native => {
            crate::linux::backend::uninstall(&entry, system, sink).await?;
            for p in &entry.desktop_files {
                if p.exists() {
                    let _ = std::fs::remove_file(p);
                }
            }
            if let Some(icon) = &entry.icon_path {
                let _ = std::fs::remove_file(icon);
            }
            desktop::refresh_desktop_database();
        }
    }

    let mut registry = Registry::load();
    registry.remove(id);
    registry
        .save()
        .map_err(|e| JobError::new(format!("Couldn't update the registry: {e}")))?;
    narrate(sink, format!("{} removed", entry.name));
    Ok(entry)
}

pub async fn repair(id: &str, system: &SystemInfo, sink: &Sink) -> JobResult<InstalledEntry> {
    let registry = Registry::load();
    let entry = registry
        .find(id)
        .cloned()
        .ok_or_else(|| JobError::new("That entry is no longer in the library."))?;
    narrate(sink, format!("Re-creating shortcuts for {}", entry.name));
    let fuse_ok = system.has_tool("libfuse.so.2");
    let updated = match entry.install_kind {
        InstallKind::AppImage => crate::linux::appimage::repair(&entry, false, fuse_ok)
            .map_err(|e| JobError::new(e.to_string()))?,
        InstallKind::Native => crate::linux::backend::repair(&entry, sink).await?,
    };
    let mut registry = Registry::load();
    registry.upsert(updated.clone());
    registry
        .save()
        .map_err(|e| JobError::new(format!("Couldn't update the registry: {e}")))?;
    narrate(sink, "Shortcuts repaired");
    Ok(updated)
}

pub fn launch(entry: &InstalledEntry) -> Result<(), String> {
    let desktop_file = entry
        .launcher
        .iter()
        .chain(entry.desktop_files.iter())
        .find(|p| p.exists())
        .cloned()
        .or_else(|| {
            let p = desktop::menu_entry_path(&entry.slug);
            p.exists().then_some(p)
        })
        .ok_or_else(|| "No menu entry to launch from — try Repair shortcuts.".to_string())?;

    if let Some(gio) = which("gio") {
        if spawn_detached(&gio, &["launch", &desktop_file.to_string_lossy()]) {
            return Ok(());
        }
    }
    if let Some(gtk_launch) = which("gtk-launch") {
        let id = desktop_file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if spawn_detached(&gtk_launch, &[&id]) {
            return Ok(());
        }
    }

    let raw = std::fs::read_to_string(&desktop_file).map_err(|e| e.to_string())?;
    let exec = desktop::parse("x", &raw)
        .and_then(|e| e.exec)
        .ok_or("The menu entry has no Exec line.")?;
    let parts: Vec<String> = shell_words(&exec)
        .into_iter()
        .filter(|a| !a.starts_with('%'))
        .collect();
    let (prog, args) = parts.split_first().ok_or("Empty Exec line.")?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if spawn_detached(Path::new(prog), &args) {
        Ok(())
    } else {
        Err(format!("Couldn't start {prog}."))
    }
}

fn spawn_detached(program: &Path, args: &[&str]) -> bool {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    cmd.process_group(0);
    cmd.spawn().is_ok()
}

pub fn shell_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = s.chars().peekable();
    let mut has = false;
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            (Some(_), c) => cur.push(c),
            (None, '"') | (None, '\'') => {
                quote = Some(c);
                has = true;
            }
            (None, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                    has = true;
                }
            }
            (None, c) if c.is_whitespace() => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, c) => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        assert_eq!(
            shell_words("\"/a b/App.AppImage\" --no-sandbox %U"),
            vec!["/a b/App.AppImage", "--no-sandbox", "%U"]
        );
        assert_eq!(shell_words("prog 'x y' z"), vec!["prog", "x y", "z"]);
    }
}

pub async fn install_tool(package: &str, system: &SystemInfo, sink: &Sink) -> JobResult<()> {
    if package.is_empty()
        || !package
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.+".contains(c))
    {
        return Err(JobError::new("That doesn't look like a package name."));
    }
    let (bin, args): (std::path::PathBuf, Vec<String>) = match system.family {
        crate::linux::distro::Family::Arch => (
            which("pacman").ok_or_else(|| JobError::new("pacman isn't installed?!"))?,
            vec![
                "-S".into(),
                "--noconfirm".into(),
                "--needed".into(),
                package.into(),
            ],
        ),
        crate::linux::distro::Family::Debian => {
            let apt = which("apt-get").ok_or_else(|| JobError::new("apt-get isn't installed?!"))?;
            (
                which("env").unwrap_or_else(|| "/usr/bin/env".into()),
                vec![
                    "DEBIAN_FRONTEND=noninteractive".into(),
                    apt.to_string_lossy().into_owned(),
                    "install".into(),
                    "-y".into(),
                    package.into(),
                ],
            )
        }
        crate::linux::distro::Family::Fedora => (
            which("dnf")
                .or_else(|| which("dnf5"))
                .ok_or_else(|| JobError::new("dnf isn't installed?!"))?,
            vec!["install".into(), "-y".into(), package.into()],
        ),
        crate::linux::distro::Family::Suse => (
            which("zypper").ok_or_else(|| JobError::new("zypper isn't installed?!"))?,
            vec!["--non-interactive".into(), "install".into(), package.into()],
        ),
        crate::linux::distro::Family::Unknown => {
            return Err(JobError::new(
                "Unknown distribution — install it with your package manager.",
            ))
        }
    };
    narrate(
        sink,
        format!("Installing {package} with administrator rights"),
    );
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = crate::linux::backend::run_root_simple(system, sink, &bin, &arg_refs).await?;
    if !out.success {
        return Err(JobError::new(format!(
            "Couldn't install {package} (exit {}). See the log for details.",
            out.code.unwrap_or(-1)
        )));
    }
    narrate(sink, format!("{package} installed"));
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BulkResult {
    pub removed: Vec<String>,

    pub failed: Vec<(String, String)>,
}

pub async fn uninstall_many(ids: &[String], system: &SystemInfo, sink: &Sink) -> BulkResult {
    let registry = Registry::load();
    let mut result = BulkResult::default();
    let mut natives: Vec<InstalledEntry> = Vec::new();

    for id in ids {
        let Some(entry) = registry.find(id).cloned() else {
            result
                .failed
                .push((id.clone(), "no longer in the library".into()));
            continue;
        };
        match entry.install_kind {
            InstallKind::AppImage => {
                narrate(sink, format!("Removing {}", entry.name));
                match crate::linux::appimage::remove(&entry) {
                    Ok(_) => result.removed.push(entry.id.clone()),
                    Err(e) => result.failed.push((entry.id.clone(), e.to_string())),
                }
            }
            InstallKind::Native => natives.push(entry),
        }
    }

    if !natives.is_empty() {
        let packages: Vec<String> = natives
            .iter()
            .filter_map(|e| e.host_package.clone())
            .collect();
        match crate::linux::backend::remove_packages(&packages, system, sink).await {
            Ok(()) => {
                for entry in &natives {
                    for p in &entry.desktop_files {
                        let _ = std::fs::remove_file(p);
                    }
                    if let Some(icon) = &entry.icon_path {
                        let _ = std::fs::remove_file(icon);
                    }
                    result.removed.push(entry.id.clone());
                }
                desktop::refresh_desktop_database();
            }
            Err(e) => {
                for entry in &natives {
                    result.failed.push((entry.id.clone(), e.message.clone()));
                }
            }
        }
    }

    if !result.removed.is_empty() {
        let mut registry = Registry::load();
        registry.entries.retain(|e| !result.removed.contains(&e.id));
        if let Err(e) = registry.save() {
            narrate(sink, format!("Couldn't update the registry: {e}"));
        }
        narrate(
            sink,
            format!(
                "Removed {} item{}",
                result.removed.len(),
                if result.removed.len() == 1 { "" } else { "s" }
            ),
        );
    }
    result
}

fn edit_entry(id: &str, f: impl FnOnce(&mut InstalledEntry)) -> Result<InstalledEntry, String> {
    let mut registry = Registry::load();
    let entry = registry
        .entries
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or("That entry is no longer in the library.")?;
    f(entry);
    let updated = entry.clone();
    registry
        .save()
        .map_err(|e| format!("Couldn't update the registry: {e}"))?;
    Ok(updated)
}

pub fn set_update_dir(id: &str, dir: Option<std::path::PathBuf>) -> Result<InstalledEntry, String> {
    if let Some(d) = &dir {
        if !d.is_dir() {
            return Err(format!("{} is not a folder.", d.display()));
        }
    }
    edit_entry(id, |e| e.update_dir = dir)
}

pub fn ignore_version(id: &str, version: &str) -> Result<InstalledEntry, String> {
    edit_entry(id, |e| {
        if !e.ignored_versions.iter().any(|v| v == version) {
            e.ignored_versions.push(version.to_string());
        }
    })
}

pub fn has_desktop_shortcut(entry: &InstalledEntry) -> bool {
    let dir = desktop::desktop_dir();
    entry
        .desktop_files
        .iter()
        .any(|p| p.starts_with(&dir) && p.exists())
}

pub fn set_desktop_shortcut(id: &str, want: bool) -> Result<InstalledEntry, String> {
    let registry = Registry::load();
    let entry = registry
        .find(id)
        .cloned()
        .ok_or("That entry is no longer in the library.")?;
    let dir = desktop::desktop_dir();
    let existing: Vec<std::path::PathBuf> = entry
        .desktop_files
        .iter()
        .filter(|p| p.starts_with(&dir))
        .cloned()
        .collect();

    if want {
        if existing.iter().any(|p| p.exists()) {
            return Ok(entry);
        }
        let launcher = entry
            .launcher
            .iter()
            .chain(entry.desktop_files.iter())
            .find(|p| p.exists())
            .cloned()
            .ok_or("No menu entry to copy — try Repair shortcuts first.")?;
        let text = std::fs::read_to_string(&launcher).map_err(|e| e.to_string())?;
        let path = desktop::write_desktop_shortcut(&entry.slug, &text)
            .map_err(|e| format!("Couldn't write the desktop shortcut: {e}"))?;
        edit_entry(id, |e| {
            e.desktop_files.retain(|p| !p.starts_with(&dir));
            e.desktop_files.push(path);
        })
    } else {
        for p in &existing {
            let _ = std::fs::remove_file(p);
        }
        edit_entry(id, |e| e.desktop_files.retain(|p| !p.starts_with(&dir)))
    }
}

pub fn scan_updates() -> Vec<crate::linux::update::UpdateCandidate> {
    let registry = Registry::load();
    let settings = crate::linux::settings::Settings::load();
    crate::linux::update::scan(&registry, &settings)
}
