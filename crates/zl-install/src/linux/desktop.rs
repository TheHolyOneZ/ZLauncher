use std::path::Path;

use crate::linux::model::DesktopEntry;

pub fn parse(path_in_package: &str, raw: &str) -> Option<DesktopEntry> {
    let mut in_group = false;
    let mut name = None;
    let mut exec = None;
    let mut icon = None;
    let mut comment = None;
    let mut categories = Vec::new();
    let mut no_display = false;
    let mut is_application = true;
    let mut seen_group = false;

    for line in raw.lines() {
        let line = line.trim_end();
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            seen_group |= in_group;
            continue;
        }
        if !in_group || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "Name" => name = Some(v.to_string()),
            "Exec" => exec = Some(v.to_string()),
            "Icon" => icon = Some(v.to_string()),
            "Comment" => comment = Some(v.to_string()),
            "Categories" => {
                categories = v
                    .split(';')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            }
            "NoDisplay" => no_display = v.eq_ignore_ascii_case("true"),
            "Type" => is_application = v == "Application",
            _ => {}
        }
    }

    if !seen_group || !is_application {
        return None;
    }
    Some(DesktopEntry {
        path: path_in_package.to_string(),
        name: name.unwrap_or_else(|| {
            Path::new(path_in_package)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        }),
        exec,
        icon,
        comment,
        categories,
        no_display,
        raw: raw.to_string(),
    })
}

pub fn set_key(raw: &str, key: &str, value: &str) -> String {
    let mut out = Vec::new();
    let mut in_group = false;
    let mut done = false;
    let mut group_start: Option<usize> = None;

    for line in raw.lines() {
        if line.trim_end().starts_with('[') {
            if in_group && !done {
                out.push(format!("{key}={value}"));
                done = true;
            }
            in_group = line.trim_end() == "[Desktop Entry]";
            out.push(line.to_string());
            if in_group {
                group_start = Some(out.len());
            }
            continue;
        }
        if in_group && !done {
            if let Some((k, _)) = line.split_once('=') {
                if k.trim() == key {
                    out.push(format!("{key}={value}"));
                    done = true;
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }
    if !done {
        match group_start {
            Some(start) => {
                let next_group = out[start..]
                    .iter()
                    .position(|l| l.trim_end().starts_with('['))
                    .map(|i| start + i);
                let mut at = next_group.unwrap_or(out.len());
                while at > start && out[at - 1].trim().is_empty() {
                    at -= 1;
                }
                out.insert(at, format!("{key}={value}"));
            }
            None => {
                out.insert(0, "[Desktop Entry]".to_string());
                out.insert(1, format!("{key}={value}"));
            }
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

pub fn remove_key(raw: &str, key: &str) -> String {
    let mut in_group = false;
    let mut s = raw
        .lines()
        .filter(|line| {
            if line.trim_end().starts_with('[') {
                in_group = line.trim_end() == "[Desktop Entry]";
                return true;
            }
            !(in_group
                && line
                    .split_once('=')
                    .map(|(k, _)| k.trim() == key)
                    .unwrap_or(false))
        })
        .collect::<Vec<_>>()
        .join("\n");
    s.push('\n');
    s
}

pub fn rewrite_exec(exec: &str, new_program: &Path) -> String {
    let program = format!("\"{}\"", new_program.display());
    let rest = split_exec_args(exec);
    if rest.is_empty() {
        program
    } else {
        format!("{program} {rest}")
    }
}

fn split_exec_args(exec: &str) -> String {
    let exec = exec.trim_start();
    if let Some(stripped) = exec.strip_prefix('"') {
        match stripped.find('"') {
            Some(end) => stripped[end + 1..].trim().to_string(),
            None => String::new(),
        }
    } else {
        exec.split_once(char::is_whitespace)
            .map(|(_, r)| r.trim().to_string())
            .unwrap_or_default()
    }
}

pub fn exec_program(exec: &str) -> String {
    let exec = exec.trim_start();
    if let Some(stripped) = exec.strip_prefix('"') {
        stripped.split('"').next().unwrap_or("").to_string()
    } else {
        exec.split_whitespace().next().unwrap_or("").to_string()
    }
}

use crate::linux::distro::which;
use crate::linux::proc::run_quietly;
use crate::linux::registry::{applications_dir, icons_dir};
use std::path::PathBuf;

pub fn menu_entry_path(slug: &str) -> PathBuf {
    applications_dir().join(format!("zlauncher-{slug}.desktop"))
}

pub fn desktop_dir() -> PathBuf {
    if let Some(bin) = which("xdg-user-dir") {
        if let Ok(out) = std::process::Command::new(bin).arg("DESKTOP").output() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() && Path::new(&s).is_dir() {
                return PathBuf::from(s);
            }
        }
    }
    dirs::desktop_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Desktop"))
}

pub fn write_icon(slug: &str, ext: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let dir = icons_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{slug}.{ext}"));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

pub fn write_menu_entry(slug: &str, contents: &str) -> std::io::Result<PathBuf> {
    let path = menu_entry_path(slug);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, contents)?;
    refresh_desktop_database();
    Ok(path)
}

pub fn refresh_desktop_database() {
    if let Some(bin) = which("update-desktop-database") {
        let dir = applications_dir();
        run_quietly(&bin, &["-q", &dir.to_string_lossy()]);
    }
}

pub fn write_desktop_shortcut(slug: &str, contents: &str) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let dir = desktop_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{slug}.desktop"));
    std::fs::write(&path, contents)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    if let Some(gio) = which("gio") {
        run_quietly(
            &gio,
            &["set", &path.to_string_lossy(), "metadata::trusted", "true"],
        );
    }
    Ok(path)
}

pub fn synthesize(name: &str, exec: &str, icon: Option<&str>, comment: Option<&str>) -> String {
    let mut s = String::from("[Desktop Entry]\nType=Application\n");
    s.push_str(&format!("Name={name}\n"));
    s.push_str(&format!("Exec={exec}\n"));
    if let Some(i) = icon {
        s.push_str(&format!("Icon={i}\n"));
    }
    if let Some(c) = comment {
        s.push_str(&format!("Comment={c}\n"));
    }
    s.push_str("Terminal=false\nCategories=Utility;\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "[Desktop Entry]\nName=Obsidian\nName[de]=Obsidian DE\nExec=AppRun %U\nIcon=obsidian\nType=Application\nCategories=Office;Utility;\n\n[Desktop Action New]\nName=New\nExec=AppRun --new\n";

    #[test]
    fn parses_main_group_only() {
        let e = parse("obsidian.desktop", SAMPLE).unwrap();
        assert_eq!(e.name, "Obsidian");
        assert_eq!(e.exec.as_deref(), Some("AppRun %U"));
        assert_eq!(e.categories, vec!["Office", "Utility"]);
        assert!(!e.no_display);
    }

    #[test]
    fn set_key_replaces_in_group_but_not_actions() {
        let out = set_key(SAMPLE, "Exec", "/x/App.AppImage %U");
        assert!(out.contains("\nExec=/x/App.AppImage %U\n"));
        assert!(out.contains("Exec=AppRun --new"));
        let out = set_key(&out, "X-ZLauncher-Id", "abc");
        assert!(out.contains("X-ZLauncher-Id=abc"));
        assert_eq!(
            parse("a", &out).unwrap().exec.as_deref(),
            Some("/x/App.AppImage %U")
        );
    }

    #[test]
    fn exec_rewrite_keeps_args() {
        assert_eq!(
            rewrite_exec("AppRun --no-sandbox %U", Path::new("/a/b.AppImage")),
            "\"/a/b.AppImage\" --no-sandbox %U"
        );
        assert_eq!(
            rewrite_exec("\"/usr/bin/some app\" %F", Path::new("/x")),
            "\"/x\" %F"
        );
        assert_eq!(rewrite_exec("AppRun", Path::new("/x")), "\"/x\"");
        assert_eq!(
            exec_program("\"/usr/bin/some app\" %F"),
            "/usr/bin/some app"
        );
    }
}
