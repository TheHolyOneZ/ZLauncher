use std::path::{Path, PathBuf};

use base64::Engine;

use crate::linux::desktop;
use crate::linux::job::{JobContext, JobResult};
use crate::linux::model::{DesktopEntry, PackageModel};

pub struct ShortcutOutcome {
    pub desktop_files: Vec<PathBuf>,

    pub launcher: Option<PathBuf>,
    pub icon_path: Option<PathBuf>,
}

fn shipped_entry(model: &PackageModel) -> Option<(&DesktopEntry, PathBuf)> {
    model
        .desktop_entries
        .iter()
        .filter(|e| !e.no_display)
        .map(|e| (e, PathBuf::from("/").join(&e.path)))
        .find(|(_, p)| p.exists())
}

pub fn guess_executable(model: &PackageModel) -> Option<PathBuf> {
    let name = model.name.to_lowercase();
    let candidates: Vec<PathBuf> = vec![
        PathBuf::from(format!("/usr/bin/{name}")),
        PathBuf::from(format!("/usr/local/bin/{name}")),
        PathBuf::from(format!("/opt/{}/{name}", model.display_name)),
        PathBuf::from(format!("/opt/{name}/{name}")),
    ];
    for c in candidates {
        if is_exec(&c) {
            return Some(c);
        }
    }

    for top in &model.top_level_paths {
        if top == "usr/bin" || top == "usr/local/bin" {
            if let Ok(rd) = std::fs::read_dir(PathBuf::from("/").join(top)) {
                let mut bins: Vec<PathBuf> = rd
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| is_exec(p))
                    .collect();
                bins.sort();
                if let Some(b) = bins.into_iter().find(|p| {
                    p.file_name()
                        .map(|f| f.to_string_lossy().to_lowercase().contains(&name))
                        .unwrap_or(false)
                }) {
                    return Some(b);
                }
            }
        }
        if let Some(rest) = top.strip_prefix("opt/") {
            let dir = PathBuf::from("/opt").join(rest);
            if let Ok(rd) = std::fs::read_dir(&dir) {
                let mut bins: Vec<PathBuf> = rd
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| is_exec(p) && p.extension().is_none())
                    .collect();
                bins.sort_by_key(|p| {
                    p.file_name()
                        .map(|f| f.to_string_lossy().to_lowercase() != name)
                        .unwrap_or(true)
                });
                if let Some(b) = bins.first() {
                    return Some(b.clone());
                }
            }
        }
    }
    None
}

pub fn looks_like_gui(path: &Path) -> bool {
    const GUI_LIBS: &[&[u8]] = &[
        b"libX11",
        b"libwayland-client",
        b"libgtk",
        b"libgdk",
        b"libQt",
        b"libSDL",
        b"libglfw",
        b"libwebkit2gtk",
        b"libEGL",
        b"libGLX",
        b"libxcb",
        b"libgbm",
        b"libnss3",
        b"libatspi",
        b"libfltk",
        b"libwx_",
    ];
    const SCRIPT_HINTS: &[&[u8]] = &[
        b"electron",
        b"java",
        b"python",
        b"chrome",
        b"xdg-open",
        b"AppRun",
        b"/opt/",
    ];
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = vec![0u8; 8 * 1024 * 1024];
    let mut total = Vec::new();
    while let Ok(n) = std::io::Read::read(&mut f, &mut buf) {
        if n == 0 || total.len() > 64 * 1024 * 1024 {
            break;
        }
        total.extend_from_slice(&buf[..n]);
    }
    if total.starts_with(b"\x7fELF") {
        GUI_LIBS.iter().any(|lib| contains(&total, lib))
    } else if total.starts_with(b"#!") {
        SCRIPT_HINTS.iter().any(|h| contains(&total, h))
    } else {
        false
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn is_exec(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn save_extracted_icon(model: &PackageModel) -> Option<PathBuf> {
    let url = model.icon_data_url.as_deref()?;
    let (head, data) = url.split_once(",")?;
    let mime = head.strip_prefix("data:")?.split(';').next()?;
    let ext = match mime {
        "image/png" => "png",
        "image/svg+xml" => "svg",
        "image/jpeg" => "jpg",
        _ => return None,
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()?;
    desktop::write_icon(&model.name, ext, &bytes).ok()
}

pub async fn ensure_shortcuts(ctx: &JobContext) -> JobResult<ShortcutOutcome> {
    let model = &ctx.plan.package;
    let mut created = Vec::new();

    let icon_path = save_extracted_icon(model);
    let mut launcher = None;

    let text = match shipped_entry(model) {
        Some((entry, path)) => {
            ctx.narrate(format!(
                "{} added itself to your app menu",
                model.display_name
            ));
            ctx.log(&format!(
                "menu entry shipped by the package: {}",
                path.display()
            ));
            launcher = Some(path.clone());
            std::fs::read_to_string(&path).unwrap_or_else(|_| entry.raw.clone())
        }
        None => {
            let Some(exec) = guess_executable(model) else {
                ctx.narrate("Couldn't find a program to make a menu entry for — it may be a command-line tool");
                return Ok(ShortcutOutcome {
                    desktop_files: created,
                    launcher,
                    icon_path,
                });
            };
            let icon_ref = icon_path.as_ref().map(|p| p.to_string_lossy().into_owned());
            let text = desktop::synthesize(
                &model.display_name,
                &format!("{} %U", desktop::rewrite_exec("x", &exec)),
                icon_ref.as_deref(),
                model.summary.as_deref(),
            );
            let text = desktop::set_key(&text, "X-ZLauncher-Id", &model.name);
            if ctx.options.menu_entry {
                let p = desktop::write_menu_entry(&model.name, &text).map_err(|e| {
                    crate::linux::job::JobError::new(format!("Couldn't write the menu entry: {e}"))
                })?;
                ctx.narrate(format!(
                    "Made a menu entry for {} ({})",
                    model.display_name,
                    exec.display()
                ));
                launcher = Some(p.clone());
                created.push(p);
            }
            text
        }
    };

    if ctx.options.desktop_shortcut {
        let p = desktop::write_desktop_shortcut(&model.name, &text).map_err(|e| {
            crate::linux::job::JobError::new(format!("Couldn't write the desktop shortcut: {e}"))
        })?;
        ctx.narrate("Put a shortcut on your desktop");
        if launcher.is_none() {
            launcher = Some(p.clone());
        }
        created.push(p);
    }
    desktop::refresh_desktop_database();
    Ok(ShortcutOutcome {
        desktop_files: created,
        launcher,
        icon_path,
    })
}
