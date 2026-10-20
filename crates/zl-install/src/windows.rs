use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};
use winreg::RegKey;
use zl_crawler::Kind;

use crate::{apps_dir, safe_name, step, Error, Event, Events, Installed, Request, Result};

const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;

#[derive(Debug, Clone, PartialEq)]
struct UninstallEntry {
    hive: String,
    path: String,
    flags: u32,
    name: String,
    version: Option<String>,
    quiet: Option<String>,
    uninstall: Option<String>,
    icon: Option<String>,
    location: Option<String>,
}

fn hive(name: &str) -> RegKey {
    RegKey::predef(if name == "HKLM" { HKEY_LOCAL_MACHINE } else { HKEY_CURRENT_USER })
}

fn read_entries() -> HashMap<String, UninstallEntry> {
    let mut out = HashMap::new();
    for (h, flags) in [("HKCU", KEY_WOW64_64KEY), ("HKLM", KEY_WOW64_64KEY), ("HKLM", KEY_WOW64_32KEY)] {
        let Ok(root) = hive(h).open_subkey_with_flags(UNINSTALL, KEY_READ | flags) else { continue };
        for sub in root.enum_keys().flatten() {
            let Ok(k) = root.open_subkey_with_flags(&sub, KEY_READ | flags) else { continue };
            let Ok(name) = k.get_value::<String, _>("DisplayName") else { continue };
            let get = |v: &str| k.get_value::<String, _>(v).ok().filter(|s| !s.trim().is_empty());
            let path = format!(r"{UNINSTALL}\{sub}");
            out.insert(
                format!("{h}|{flags}|{path}"),
                UninstallEntry {
                    hive: h.into(),
                    path,
                    flags,
                    name,
                    version: get("DisplayVersion"),
                    quiet: get("QuietUninstallString"),
                    uninstall: get("UninstallString"),
                    icon: get("DisplayIcon"),
                    location: get("InstallLocation"),
                },
            );
        }
    }
    out
}

fn read_entry(h: &str, path: &str, flags: u32) -> Option<UninstallEntry> {
    read_entries().into_values().find(|e| e.hive == h && e.path == path && e.flags == flags)
}

fn product_of(filename: &str) -> String {
    let stem = filename.split(['_']).next().unwrap_or(filename);
    stem.trim_end_matches("-setup").to_string()
}

fn find_new(before: &HashMap<String, UninstallEntry>, title: &str, filename: &str) -> Option<UninstallEntry> {
    let after = read_entries();
    let changed: Vec<&UninstallEntry> =
        after.iter().filter(|(k, e)| before.get(*k) != Some(*e)).map(|(_, e)| e).collect();
    let names = [title.to_lowercase(), product_of(filename).to_lowercase()];
    let matches = |e: &UninstallEntry| {
        let n = e.name.to_lowercase();
        names.iter().any(|t| !t.is_empty() && (n == *t || n.starts_with(t.as_str()) || t.starts_with(&n)))
    };
    changed
        .iter()
        .find(|e| matches(e))
        .or_else(|| (changed.len() == 1).then(|| &changed[0]))
        .map(|e| (*e).clone())
        .or_else(|| after.values().find(|e| matches(e)).cloned())
}

fn run_wait(program: &Path, args: &[&str]) -> Result<i32> {
    let status = Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| Error::new(format!("Couldn't start {}: {e}", program.display())))?;
    Ok(status.code().unwrap_or(-1))
}

const ERROR_ELEVATION_REQUIRED: i32 = 740;
const ERROR_CANCELLED: i32 = 1223;

fn run_or_elevate(program: &Path, args: &str, events: &Events) -> Result<i32> {
    match Command::new(program).raw_arg(args).creation_flags(CREATE_NO_WINDOW).status() {
        Ok(s) => Ok(s.code().unwrap_or(-1)),
        Err(e) if e.raw_os_error() == Some(ERROR_ELEVATION_REQUIRED) => run_elevated(&program.to_string_lossy(), args, events),
        Err(e) => Err(Error::new(format!("Couldn't start {}: {e}", program.display()))),
    }
}

fn run_elevated(program: &str, args: &str, events: &Events) -> Result<i32> {
    let _ = events.send(Event::NeedsAuth);
    let arg_list = if args.trim().is_empty() { String::new() } else { format!(" -ArgumentList '{}'", args.replace('\'', "''")) };
    let ps = format!(
        "try {{ $p = Start-Process -FilePath '{}'{arg_list} -Verb RunAs -Wait -PassThru -ErrorAction Stop; exit $p.ExitCode }} catch {{ exit 1223 }}",
        program.replace('\'', "''"),
    );
    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| Error::new(format!("Couldn't start PowerShell: {e}")))?;
    match status.code() {
        Some(0) => Ok(0),
        Some(ERROR_CANCELLED) => Err(Error::hint("Windows did not allow the change.", "Accept the administrator prompt to continue.")),
        Some(c) => Ok(c),
        None => Ok(-1),
    }
}

fn start_menu_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_default().join(r"Microsoft\Windows\Start Menu\Programs")
}

fn desktop_link(title: &str) -> Option<PathBuf> {
    dirs::desktop_dir().map(|d| d.join(format!("{}.lnk", safe_name(title))))
}

fn write_shortcut(link: &Path, target: &Path) -> Result<()> {
    if let Some(dir) = link.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let ps = format!(
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut('{}'); $s.TargetPath = '{}'; $s.WorkingDirectory = '{}'; $s.Save()",
        link.display().to_string().replace('\'', "''"),
        target.display().to_string().replace('\'', "''"),
        target.parent().map(|p| p.display().to_string()).unwrap_or_default().replace('\'', "''"),
    );
    let code = run_wait(Path::new("powershell.exe"), &["-NoProfile", "-NonInteractive", "-Command", &ps])?;
    if code == 0 {
        Ok(())
    } else {
        Err(Error::new("Couldn't create the shortcut."))
    }
}

fn exe_of(e: &UninstallEntry) -> Option<PathBuf> {
    if let Some(icon) = &e.icon {
        let p = icon.trim().trim_matches('"');
        let p = p.rsplit_once(',').filter(|(_, n)| n.trim().parse::<i32>().is_ok()).map(|(a, _)| a).unwrap_or(p);
        let p = PathBuf::from(p.trim_matches('"'));
        if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")) && p.exists() {
            return Some(p);
        }
    }
    let dir = PathBuf::from(e.location.as_deref()?.trim_matches('"'));
    let want = e.name.to_lowercase().replace(' ', "");
    let exes: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|d| d.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")))
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().starts_with("uninstall")))
        .collect();
    exes.iter()
        .find(|p| p.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase().replace(' ', "") == want))
        .or(exes.first())
        .cloned()
}

fn handle_for(e: &UninstallEntry, exe: Option<&Path>, desktop: Option<&Path>) -> serde_json::Value {
    json!({
        "hive": e.hive,
        "key": e.path,
        "flags": e.flags,
        "name": e.name,
        "exe": exe.map(|p| p.display().to_string()),
        "desktop": desktop.map(|p| p.display().to_string()),
    })
}

fn str_of<'a>(h: &'a serde_json::Value, k: &str) -> Option<&'a str> {
    h.get(k).and_then(|v| v.as_str())
}

pub async fn install(req: Request<'_>, events: Events, _cancel: Arc<AtomicBool>) -> Result<Installed> {
    let file = req.file.to_path_buf();
    let title = req.title.to_string();
    let filename = req.artifact.filename.clone();
    let kind = req.artifact.kind;
    let version = req.artifact.version.clone();
    let options = req.options;
    let id = req.artifact.project_id.clone();

    tokio::task::spawn_blocking(move || -> Result<Installed> {
        if kind == Kind::PortableExe {
            step(&events, "Copying the program");
            let dir = apps_dir().join(&id);
            std::fs::create_dir_all(&dir)?;
            let exe = dir.join(&filename);
            std::fs::copy(&file, &exe)?;
            let menu = start_menu_dir().join("ZLauncher").join(format!("{}.lnk", safe_name(&title)));
            if options.menu_entry {
                write_shortcut(&menu, &exe)?;
            }
            let desktop = if options.desktop_shortcut { desktop_link(&title) } else { None };
            if let Some(d) = &desktop {
                write_shortcut(d, &exe)?;
            }
            return Ok(Installed {
                method: "portable".into(),
                handle: json!({ "exe": exe, "dir": dir, "menu": menu, "desktop": desktop }),
                version,
            });
        }

        let before = read_entries();
        let (method, code) = if kind == Kind::Msi {
            step(&events, "Running the installer (administrator)");
            let args = format!("/i \"{}\" /qn /norestart", file.display());
            ("msi", run_elevated("msiexec.exe", &args, &events)?)
        } else {
            step(&events, "Running the installer");
            ("nsis", run_or_elevate(&file, "/S", &events)?)
        };
        if code != 0 && code != 3010 {
            return Err(Error::new(format!("The installer stopped with code {code}.")));
        }
        if code == 3010 {
            step(&events, "Installed; Windows needs a restart to finish");
        }

        let mut found = None;
        for _ in 0..20 {
            found = find_new(&before, &title, &filename);
            if found.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        let entry = found.ok_or_else(|| {
            Error::hint(
                "The installer finished, but the launcher couldn't find what it installed.",
                "It is probably installed anyway; look for it in the Start menu.",
            )
        })?;
        let exe = exe_of(&entry);
        let desktop = match (&exe, options.desktop_shortcut) {
            (Some(exe), true) => {
                let link = desktop_link(&title);
                if let Some(l) = &link {
                    if !l.exists() {
                        write_shortcut(l, exe)?;
                    }
                }
                link
            }
            _ => None,
        };
        Ok(Installed {
            method: method.into(),
            handle: handle_for(&entry, exe.as_deref(), desktop.as_deref()),
            version: entry.version.clone().or(version),
        })
    })
    .await
    .map_err(|e| Error::new(e.to_string()))?
}

fn split_command(cmd: &str) -> (String, String) {
    let cmd = cmd.trim();
    if let Some(rest) = cmd.strip_prefix('"') {
        if let Some((prog, args)) = rest.split_once('"') {
            return (prog.to_string(), args.trim().to_string());
        }
    }
    let lower = cmd.to_lowercase();
    match lower.find(".exe") {
        Some(i) => (cmd[..i + 4].to_string(), cmd[i + 4..].trim().to_string()),
        None => (cmd.to_string(), String::new()),
    }
}

pub async fn uninstall(method: &str, handle: &serde_json::Value, events: Events) -> Result<()> {
    let method = method.to_string();
    let handle = handle.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        if let Some(d) = str_of(&handle, "desktop") {
            let _ = std::fs::remove_file(d);
        }
        if method == "portable" {
            step(&events, "Removing the program");
            if let Some(m) = str_of(&handle, "menu") {
                let _ = std::fs::remove_file(m);
            }
            if let Some(d) = str_of(&handle, "dir") {
                std::fs::remove_dir_all(d)?;
            }
            return Ok(());
        }

        let (h, key) = (str_of(&handle, "hive").unwrap_or("HKCU"), str_of(&handle, "key").unwrap_or(""));
        let flags = handle.get("flags").and_then(|v| v.as_u64()).unwrap_or(KEY_WOW64_64KEY as u64) as u32;
        let Some(entry) = read_entry(h, key, flags) else {
            step(&events, "It was already removed");
            return Ok(());
        };
        step(&events, "Running the uninstaller");
        let cmd = entry.quiet.clone().or(entry.uninstall.clone()).ok_or_else(|| Error::new("This program has no uninstaller."))?;
        let (prog, mut args) = split_command(&cmd);
        let code = if prog.to_lowercase().ends_with("msiexec.exe") {
            let guid = args.split(['{', '}']).nth(1).map(|g| format!("{{{g}}}")).ok_or_else(|| Error::new("The uninstall entry names no product code."))?;
            let args = format!("/x {guid} /qn /norestart");
            if entry.hive == "HKLM" {
                run_elevated("msiexec.exe", &args, &events)?
            } else {
                run_wait(Path::new("msiexec.exe"), &args.split(' ').collect::<Vec<_>>())?
            }
        } else {
            if entry.quiet.is_none() && !args.contains("/S") {
                args = format!("{args} /S").trim().to_string();
            }
            run_or_elevate(Path::new(&prog), &args, &events)?
        };
        if code != 0 && code != 3010 {
            return Err(Error::new(format!("The uninstaller stopped with code {code}.")));
        }
        for _ in 0..60 {
            if read_entry(h, key, flags).is_none() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        Err(Error::new("The uninstaller ran, but the program is still registered."))
    })
    .await
    .map_err(|e| Error::new(e.to_string()))?
}

fn exe_for(method: &str, handle: &serde_json::Value) -> Option<PathBuf> {
    if let Some(e) = str_of(handle, "exe").map(PathBuf::from).filter(|p| p.exists()) {
        return Some(e);
    }
    if method == "portable" {
        return None;
    }
    let flags = handle.get("flags").and_then(|v| v.as_u64()).unwrap_or(KEY_WOW64_64KEY as u64) as u32;
    read_entry(str_of(handle, "hive")?, str_of(handle, "key")?, flags).and_then(|e| exe_of(&e))
}

pub fn launch(method: &str, handle: &serde_json::Value) -> Result<()> {
    let exe = exe_for(method, handle).ok_or_else(|| Error::new("Couldn't find the program to start."))?;
    Command::new(&exe)
        .current_dir(exe.parent().unwrap_or(Path::new(".")))
        .creation_flags(DETACHED_PROCESS)
        .spawn()
        .map(|_| ())
        .map_err(|e| Error::new(format!("Couldn't start {}: {e}", exe.display())))
}

pub fn is_present(method: &str, handle: &serde_json::Value) -> bool {
    if method == "portable" {
        return str_of(handle, "exe").is_some_and(|p| Path::new(p).exists());
    }
    let flags = handle.get("flags").and_then(|v| v.as_u64()).unwrap_or(KEY_WOW64_64KEY as u64) as u32;
    match (str_of(handle, "hive"), str_of(handle, "key")) {
        (Some(h), Some(k)) => read_entry(h, k, flags).is_some(),
        _ => false,
    }
}

fn title_of(handle: &serde_json::Value) -> String {
    str_of(handle, "name")
        .map(str::to_string)
        .or_else(|| str_of(handle, "exe").and_then(|e| Path::new(e).file_stem().map(|s| s.to_string_lossy().into_owned())))
        .unwrap_or_else(|| "App".into())
}

pub fn has_desktop_shortcut(_method: &str, handle: &serde_json::Value) -> bool {
    str_of(handle, "desktop").is_some_and(|d| Path::new(d).exists())
        || desktop_link(&title_of(handle)).is_some_and(|d| d.exists())
}

pub fn set_desktop_shortcut(method: &str, handle: &serde_json::Value, want: bool) -> Result<serde_json::Value> {
    let mut h = handle.clone();
    let link = str_of(handle, "desktop").map(PathBuf::from).or_else(|| desktop_link(&title_of(handle)));
    let Some(link) = link else { return Err(Error::new("There is no desktop folder.")) };
    if want {
        let exe = exe_for(method, handle).ok_or_else(|| Error::new("Couldn't find the program."))?;
        write_shortcut(&link, &exe)?;
        h["desktop"] = json!(link.display().to_string());
    } else {
        let _ = std::fs::remove_file(&link);
        h["desktop"] = serde_json::Value::Null;
    }
    Ok(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_commands() {
        assert_eq!(
            split_command(r#""C:\Program Files\ZPad\uninstall.exe" /S"#),
            (r"C:\Program Files\ZPad\uninstall.exe".into(), "/S".into())
        );
        assert_eq!(split_command("MsiExec.exe /X{ABC}"), ("MsiExec.exe".into(), "/X{ABC}".into()));
        assert_eq!(product_of("ZBase Studio_0.2.0_x64-setup.exe"), "ZBase Studio");
    }
}
