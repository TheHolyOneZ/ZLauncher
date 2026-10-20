use std::collections::HashMap;

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct External {
    pub version: Option<String>,
    pub via: String,
    pub name: String,
}

pub fn kebab(product: &str) -> String {
    let chars: Vec<char> = product.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !out.ends_with('-') && !out.is_empty() {
                out.push('-');
            }
            continue;
        }
        if c.is_uppercase() && i > 0 {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if (prev.is_lowercase() || prev.is_ascii_digit() || (prev.is_uppercase() && next_lower)) && !out.ends_with('-') {
                out.push('-');
            }
        }
        out.extend(c.to_lowercase());
    }
    out.trim_matches('-').to_string()
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

pub fn product_of(filename: &str) -> String {
    let cut = filename
        .char_indices()
        .find(|&(i, c)| (c == '_' || c == '-') && filename[i + 1..].chars().next().is_some_and(|n| n.is_ascii_digit()))
        .map(|(i, _)| i)
        .or_else(|| filename.find('_'))
        .unwrap_or_else(|| filename.find('.').unwrap_or(filename.len()));
    filename[..cut].to_string()
}

#[cfg(not(windows))]
fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program).args(args).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn system_packages() -> HashMap<String, (String, Option<String>, &'static str)> {
    let mut map = HashMap::new();
    #[cfg(target_os = "linux")]
    {
        let mut add = |text: String, via: &'static str, split: fn(&str) -> Option<(&str, &str)>| {
            for line in text.lines() {
                if let Some((n, v)) = split(line) {
                    let v = v.split('-').next().unwrap_or(v);
                    let v = v.rsplit(':').next().unwrap_or(v);
                    map.entry(n.to_lowercase()).or_insert((n.to_string(), Some(v.to_string()), via));
                }
            }
        };
        if let Some(t) = run("pacman", &["-Q"]) {
            add(t, "pacman", |l| l.split_once(' '));
        }
        if let Some(t) = run("dpkg-query", &["-W", "-f", "${Package}\t${Version}\n"]) {
            add(t, "dpkg", |l| l.split_once('\t'));
        }
        if let Some(t) = run("rpm", &["-qa", "--qf", "%{NAME}\t%{VERSION}\n"]) {
            add(t, "rpm", |l| l.split_once('\t'));
        }
        if let Some(dir) = dirs::home_dir().map(|h| h.join("Applications")) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if let Some(stem) = name.strip_suffix(".AppImage").or_else(|| name.strip_suffix(".appimage")) {
                    let product = product_of(stem);
                    let version = zl_crawler::classify::version_of(stem);
                    map.entry(format!("appimage:{}", norm(&product))).or_insert((name.clone(), version, "appimage"));
                }
            }
        }
    }
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};
        let key = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
        for (hive, flags) in [(HKEY_CURRENT_USER, KEY_WOW64_64KEY), (HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY), (HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY)] {
            let Ok(root) = winreg::RegKey::predef(hive).open_subkey_with_flags(key, KEY_READ | flags) else { continue };
            for sub in root.enum_keys().flatten() {
                let Ok(k) = root.open_subkey_with_flags(&sub, KEY_READ | flags) else { continue };
                let Ok(name) = k.get_value::<String, _>("DisplayName") else { continue };
                let version = k.get_value::<String, _>("DisplayVersion").ok();
                map.entry(format!("win:{}", norm(&name))).or_insert((name, version, "registry"));
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let dirs = [std::path::PathBuf::from("/Applications"), dirs::home_dir().unwrap_or_default().join("Applications")];
        for d in dirs {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if let Some(stem) = name.strip_suffix(".app") {
                    let plist = e.path().join("Contents/Info.plist");
                    let version = run("/usr/bin/plutil", &["-extract", "CFBundleShortVersionString", "raw", "-o", "-", &plist.to_string_lossy()])
                        .map(|s| s.trim().to_string());
                    map.entry(format!("mac:{}", norm(stem))).or_insert((name.clone(), version, "app"));
                }
            }
        }
    }
    map
}

pub fn scan(products: &HashMap<String, Vec<String>>) -> HashMap<String, External> {
    let installed = system_packages();
    let mut out = HashMap::new();
    for (id, names) in products {
        let found = names.iter().find_map(|product| {
            let keys = if cfg!(windows) {
                vec![format!("win:{}", norm(product))]
            } else if cfg!(target_os = "macos") {
                vec![format!("mac:{}", norm(product))]
            } else {
                vec![kebab(product), product.to_lowercase(), norm(product), format!("appimage:{}", norm(product))]
            };
            keys.iter().find_map(|k| installed.get(k))
        });
        if let Some((name, version, via)) = found {
            out.insert(id.clone(), External { version: version.clone(), via: (*via).to_string(), name: name.clone() });
        }
    }
    out
}

pub fn launch(found: &External, title: &str) -> crate::Result<()> {
    let wanted = [norm(title), norm(&found.name), norm(&kebab(title))];
    #[cfg(target_os = "linux")]
    {
        if found.via == "appimage" {
            let p = dirs::home_dir().unwrap_or_default().join("Applications").join(&found.name);
            return spawn(&p.to_string_lossy(), &[]);
        }
        let dirs = [
            std::path::PathBuf::from("/usr/share/applications"),
            std::path::PathBuf::from("/usr/local/share/applications"),
            dirs::data_dir().unwrap_or_default().join("applications"),
        ];
        for d in dirs {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let path = e.path();
                if path.extension().is_none_or(|x| x != "desktop") {
                    continue;
                }
                let stem = norm(&path.file_stem().unwrap_or_default().to_string_lossy());
                let name = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|t| t.lines().find_map(|l| l.strip_prefix("Name=").map(norm)))
                    .unwrap_or_default();
                if wanted.contains(&stem) || wanted.contains(&name) {
                    let id = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    if spawn("gtk-launch", &[&id]).is_ok() {
                        return Ok(());
                    }
                    return spawn("gio", &["launch", &path.to_string_lossy()]);
                }
            }
        }
        Err(crate::Error::new("Couldn't find how to start it. Look for it in your app menu."))
    }
    #[cfg(windows)]
    {
        let roots = [dirs::data_dir().map(|d| d.join(r"Microsoft\Windows\Start Menu\Programs")), std::env::var_os("ProgramData").map(|p| std::path::PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs"))];
        let mut stack: Vec<std::path::PathBuf> = roots.into_iter().flatten().collect();
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) && wanted.contains(&norm(&p.file_stem().unwrap_or_default().to_string_lossy())) {
                    return spawn("explorer.exe", &[&p.to_string_lossy()]);
                }
            }
        }
        Err(crate::Error::new("Couldn't find its Start menu entry."))
    }
    #[cfg(target_os = "macos")]
    {
        let _ = wanted;
        spawn("/usr/bin/open", &["-a", found.name.trim_end_matches(".app")])
    }
}

fn spawn(program: &str, args: &[&str]) -> crate::Result<()> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.spawn().map(|_| ()).map_err(|e| crate::Error::new(format!("Couldn't start {program}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kebab_like_tauri() {
        assert_eq!(kebab("ZEncrypt"), "z-encrypt");
        assert_eq!(kebab("ZBase Studio"), "z-base-studio");
        assert_eq!(kebab("zhell"), "zhell");
        assert_eq!(kebab("ZIT-Suite"), "zit-suite");
        assert_eq!(kebab("ZFontManager"), "z-font-manager");
        assert_eq!(kebab("Clipboard Manager"), "clipboard-manager");
    }

    #[test]
    fn product_names() {
        assert_eq!(product_of("ZBase Studio_0.2.0_amd64.deb"), "ZBase Studio");
        assert_eq!(product_of("ZEncrypt-0.1.0-1.x86_64.rpm"), "ZEncrypt");
        assert_eq!(product_of("ZIT-Suite_0.2.1_amd64.deb"), "ZIT-Suite");
        assert_eq!(product_of("Zyringe.AppImage"), "Zyringe");
    }
}
