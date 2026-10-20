use std::path::{Path, PathBuf};

use crate::linux::desktop;
use crate::linux::inspect::appimage::read_contents;
use crate::linux::job::{JobContext, JobError, JobResult};
use crate::linux::model::{InstallKind, InstalledEntry, StageId};
use crate::linux::registry::{now_rfc3339, Registry};
use crate::linux::util::{icon_size_rank, image_mime};

fn file_stem_for(display_name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in display_name.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let t = out.trim_matches('-').to_string();
    if t.is_empty() {
        "App".into()
    } else {
        t
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (a.metadata(), b.metadata()) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

fn place_file(src: &Path, dest: &Path, keep_original: bool) -> std::io::Result<()> {
    if same_file(src, dest) {
        return Ok(());
    }
    if keep_original {
        std::fs::copy(src, dest)?;
        return Ok(());
    }
    match std::fs::rename(src, dest) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            std::fs::copy(src, dest)?;
            std::fs::remove_file(src)
        }
        Err(e) => Err(e),
    }
}

fn pick_icon(
    scan: &crate::linux::inspect::PayloadScan,
    icon_name: Option<&str>,
) -> Option<(&'static str, Vec<u8>)> {
    let wanted = icon_name.map(|s| s.trim_start_matches('/').to_string());
    let matches = |p: &str| match &wanted {
        Some(w) if w.contains('/') => p == w,
        Some(w) => Path::new(p)
            .file_stem()
            .map(|s| s.to_string_lossy().eq_ignore_ascii_case(w))
            .unwrap_or(false),
        None => false,
    };
    let candidates: Vec<&(String, Vec<u8>)> = scan
        .icons
        .iter()
        .filter(|(p, _)| image_mime(p).is_some())
        .collect();
    let best = candidates
        .iter()
        .filter(|(p, _)| matches(p))
        .max_by_key(|(p, _)| icon_size_rank(p))
        .or_else(|| {
            candidates
                .iter()
                .filter(|(p, _)| p.starts_with("diricon/"))
                .max_by_key(|(p, _)| icon_size_rank(p))
        })
        .or_else(|| candidates.iter().max_by_key(|(p, _)| icon_size_rank(p)))?;
    let ext = match image_mime(&best.0)? {
        "image/png" => "png",
        "image/svg+xml" => "svg",
        "image/jpeg" => "jpg",
        _ => "png",
    };
    Some((ext, best.1.clone()))
}

pub fn build_desktop_entry(
    ctx_name: &str,
    raw: Option<&str>,
    appimage_path: &Path,
    icon_path: Option<&Path>,
    id: &str,
    fuse_ok: bool,
    comment: Option<&str>,
) -> String {
    let base = match raw {
        Some(r) => r.to_string(),
        None => desktop::synthesize(ctx_name, "AppRun %U", None, comment),
    };
    let exec_line = desktop::parse("x.desktop", &base)
        .and_then(|e| e.exec)
        .unwrap_or_else(|| "AppRun %U".into());
    let mut exec = desktop::rewrite_exec(&exec_line, appimage_path);
    if !fuse_ok {
        let (prog, rest) = exec
            .split_once("\" ")
            .map(|(a, b)| (format!("{a}\""), b.to_string()))
            .unwrap_or((exec.clone(), String::new()));
        exec = if rest.is_empty() {
            format!("{prog} --appimage-extract-and-run")
        } else {
            format!("{prog} --appimage-extract-and-run {rest}")
        };
    }
    let mut out = desktop::set_key(&base, "Exec", &exec);
    out = desktop::set_key(&out, "TryExec", &appimage_path.to_string_lossy());
    if let Some(icon) = icon_path {
        out = desktop::set_key(&out, "Icon", &icon.to_string_lossy());
    }
    out = desktop::set_key(&out, "X-ZLauncher-Id", id);
    out = desktop::remove_key(&out, "X-AppImage-Integrate");
    if !out.contains("\nType=") {
        out = desktop::set_key(&out, "Type", "Application");
    }
    out
}

pub async fn integrate(ctx: &JobContext) -> JobResult<InstalledEntry> {
    let model = &ctx.plan.package;
    let src = model.source_path.clone();

    ctx.begin(StageId::Inspect);
    ctx.narrate(format!("Checking {}…", model.display_name));
    if !src.exists() {
        return Err(JobError::new(format!(
            "{} is gone — was it moved or deleted?",
            src.display()
        )));
    }
    let contents = tokio::task::spawn_blocking({
        let src = src.clone();
        move || read_contents(&src)
    })
    .await
    .map_err(|e| JobError::new(e.to_string()))?
    .map_err(|e| JobError::new(e.to_string()))?;
    ctx.check_cancelled()?;
    ctx.finish(StageId::Inspect);

    let registry = Registry::load();
    let previous = registry.find_by_slug(&model.name).cloned();
    let id = previous
        .as_ref()
        .map(|p| p.id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    ctx.begin(StageId::Move);
    let dir = ctx.settings.appimage_dir.clone();
    std::fs::create_dir_all(&dir).map_err(|e| {
        JobError::with_hint(
            format!("Couldn't create {}: {e}", dir.display()),
            "Pick a different AppImage folder in Settings.",
        )
    })?;
    let stem = file_stem_for(&model.display_name);
    let mut dest = dir.join(format!("{stem}.AppImage"));
    let ours = previous.as_ref().and_then(|p| p.appimage_path.clone());
    if dest.exists() && !same_file(&src, &dest) && ours.as_deref() != Some(dest.as_path()) {
        dest = dir.join(format!("{stem}-{}.AppImage", model.version));
    }

    let keep_original = !ctx.options.remove_original;
    if same_file(&src, &dest) {
        ctx.narrate(format!(
            "{} is already in {}",
            model.display_name,
            short(&dir)
        ));
    } else {
        ctx.narrate(format!(
            "{} {} into {}",
            if keep_original { "Copying" } else { "Moving" },
            model.display_name,
            short(&dir)
        ));
        let (s, d) = (src.clone(), dest.clone());
        tokio::task::spawn_blocking(move || place_file(&s, &d, keep_original))
            .await
            .map_err(|e| JobError::new(e.to_string()))?
            .map_err(|e| JobError::new(format!("Couldn't move the AppImage: {e}")))?;
    }
    if let Some(old) = ours {
        if old != dest && old.exists() && !same_file(&old, &src) {
            let _ = std::fs::remove_file(&old);
            ctx.log(&format!("removed previous version {}", old.display()));
        }
    }
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| JobError::new(format!("Couldn't make it executable: {e}")))?;
    }
    ctx.narrate("Made it executable");
    ctx.check_cancelled()?;
    ctx.finish(StageId::Move);

    ctx.begin(StageId::Shortcuts);
    let entry = contents.scan.primary_desktop("").cloned();
    let icon_path = match pick_icon(
        &contents.scan,
        entry.as_ref().and_then(|e| e.icon.as_deref()),
    ) {
        Some((ext, bytes)) => {
            let p = desktop::write_icon(&model.name, ext, &bytes)
                .map_err(|e| JobError::new(format!("Couldn't save the icon: {e}")))?;
            ctx.log(&format!("icon → {}", p.display()));
            Some(p)
        }
        None => {
            ctx.narrate("No icon inside the AppImage; the menu will show a generic one");
            None
        }
    };

    let fuse_ok = ctx.system.has_tool("libfuse.so.2");
    if !fuse_ok {
        ctx.narrate("libfuse2 is missing, so the launcher uses extract-and-run mode");
    }
    let desktop_text = build_desktop_entry(
        &model.display_name,
        entry.as_ref().map(|e| e.raw.as_str()),
        &dest,
        icon_path.as_deref(),
        &id,
        fuse_ok,
        model.summary.as_deref(),
    );

    let mut desktop_files = Vec::new();
    if ctx.options.menu_entry {
        let p = desktop::write_menu_entry(&model.name, &desktop_text)
            .map_err(|e| JobError::new(format!("Couldn't write the menu entry: {e}")))?;
        ctx.narrate(format!("Added {} to your app menu", model.display_name));
        ctx.log(&format!("menu entry → {}", p.display()));
        desktop_files.push(p);
    }
    if ctx.options.desktop_shortcut {
        let p = desktop::write_desktop_shortcut(&model.name, &desktop_text)
            .map_err(|e| JobError::new(format!("Couldn't write the desktop shortcut: {e}")))?;
        ctx.narrate("Put a shortcut on your desktop");
        desktop_files.push(p);
    }

    if let Some(prev) = &previous {
        for old in &prev.desktop_files {
            if !desktop_files.contains(old) && old.exists() {
                let _ = std::fs::remove_file(old);
            }
        }
    }
    ctx.finish(StageId::Shortcuts);

    Ok(InstalledEntry {
        id,
        name: model.display_name.clone(),
        slug: model.name.clone(),
        version: model.version.clone(),
        source_kind: model.kind,
        install_kind: InstallKind::AppImage,
        host_package: None,
        appimage_path: Some(dest),
        launcher: desktop_files.first().cloned(),
        desktop_files,
        icon_path,
        original_path: src,
        installed_at: now_rfc3339(),
        unresolved_deps: Vec::new(),
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

pub fn remove(entry: &InstalledEntry) -> std::io::Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    for p in entry
        .desktop_files
        .iter()
        .chain(entry.icon_path.iter())
        .chain(entry.appimage_path.iter())
    {
        if p.exists() {
            std::fs::remove_file(p)?;
            removed.push(p.clone());
        }
    }
    desktop::refresh_desktop_database();
    Ok(removed)
}

pub fn repair(
    entry: &InstalledEntry,
    want_desktop_shortcut: bool,
    fuse_ok: bool,
) -> std::io::Result<InstalledEntry> {
    let Some(appimage) = entry.appimage_path.clone() else {
        return Err(std::io::Error::other("not an AppImage entry"));
    };
    if !appimage.exists() {
        return Err(std::io::Error::other(format!(
            "{} no longer exists",
            appimage.display()
        )));
    }
    let contents = read_contents(&appimage).map_err(std::io::Error::other)?;
    let desktop_entry = contents.scan.primary_desktop("").cloned();
    let icon_path = match pick_icon(
        &contents.scan,
        desktop_entry.as_ref().and_then(|e| e.icon.as_deref()),
    ) {
        Some((ext, bytes)) => Some(desktop::write_icon(&entry.slug, ext, &bytes)?),
        None => entry.icon_path.clone(),
    };
    let text = build_desktop_entry(
        &entry.name,
        desktop_entry.as_ref().map(|e| e.raw.as_str()),
        &appimage,
        icon_path.as_deref(),
        &entry.id,
        fuse_ok,
        None,
    );
    let mut desktop_files = vec![desktop::write_menu_entry(&entry.slug, &text)?];
    if want_desktop_shortcut
        || entry
            .desktop_files
            .iter()
            .any(|p| p.starts_with(desktop::desktop_dir()))
    {
        desktop_files.push(desktop::write_desktop_shortcut(&entry.slug, &text)?);
    }
    Ok(InstalledEntry {
        desktop_files,
        icon_path,
        ..entry.clone()
    })
}

fn short(p: &Path) -> String {
    match dirs::home_dir() {
        Some(h) if p.starts_with(&h) => format!("~/{}", p.strip_prefix(&h).unwrap_or(p).display()),
        _ => p.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_stems() {
        assert_eq!(file_stem_for("ZRepo Manager (beta)"), "ZRepo-Manager-beta");
        assert_eq!(file_stem_for("Obsidian"), "Obsidian");
    }

    #[test]
    fn desktop_entry_rewrite() {
        let raw = "[Desktop Entry]\nName=Obsidian\nExec=AppRun --no-sandbox %U\nIcon=obsidian\nType=Application\nX-AppImage-Integrate=false\n";
        let out = build_desktop_entry(
            "Obsidian",
            Some(raw),
            Path::new("/home/u/Applications/Obsidian.AppImage"),
            Some(Path::new(
                "/home/u/.local/share/icons/zlauncher/obsidian.png",
            )),
            "id1",
            true,
            None,
        );
        assert!(out.contains("Exec=\"/home/u/Applications/Obsidian.AppImage\" --no-sandbox %U\n"));
        assert!(out.contains("TryExec=/home/u/Applications/Obsidian.AppImage\n"));
        assert!(out.contains("Icon=/home/u/.local/share/icons/zlauncher/obsidian.png\n"));
        assert!(out.contains("X-ZLauncher-Id=id1"));
        assert!(!out.contains("X-AppImage-Integrate"));
        let nofuse = build_desktop_entry(
            "Obsidian",
            Some(raw),
            Path::new("/a/O.AppImage"),
            None,
            "id",
            false,
            None,
        );
        assert!(
            nofuse.contains("Exec=\"/a/O.AppImage\" --appimage-extract-and-run --no-sandbox %U\n")
        );
    }
}
