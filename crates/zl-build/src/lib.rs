use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

pub type Log = UnboundedSender<String>;

#[derive(Debug, Clone, thiserror::Error, Serialize)]
#[error("{message}")]
pub struct Error {
    pub message: String,
    pub hint: Option<String>,
}

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), hint: None }
    }
    fn hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self { message: message.into(), hint: Some(hint.into()) }
    }
    fn cancelled() -> Self {
        Self::new("Stopped.")
    }
    pub fn is_cancelled(&self) -> bool {
        self.message == "Stopped."
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    Pnpm,
    Npm,
    Yarn,
    Bun,
}

impl PackageManager {
    pub fn program(self) -> &'static str {
        match self {
            Self::Pnpm => "pnpm",
            Self::Npm => "npm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
        }
    }

    fn install_args(self, frozen: bool) -> Vec<&'static str> {
        match (self, frozen) {
            (Self::Pnpm, true) => vec!["install", "--frozen-lockfile"],
            (Self::Pnpm, false) => vec!["install"],
            (Self::Npm, true) => vec!["ci"],
            (Self::Npm, false) => vec!["install"],
            (Self::Yarn, true) => vec!["install", "--frozen-lockfile"],
            (Self::Yarn, false) => vec!["install"],
            (Self::Bun, true) => vec!["install", "--frozen-lockfile"],
            (Self::Bun, false) => vec!["install"],
        }
    }

    fn tauri(self) -> (&'static str, Vec<&'static str>) {
        match self {
            Self::Pnpm => ("pnpm", vec!["exec", "tauri"]),
            Self::Npm => ("npx", vec!["--no-install", "tauri"]),
            Self::Yarn => ("yarn", vec!["tauri"]),
            Self::Bun => ("bunx", vec!["tauri"]),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub root: PathBuf,
    pub app_dir: PathBuf,
    pub pm: PackageManager,
    pub product_name: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Bundle {
    Deb,
    Rpm,
    Appimage,
    Nsis,
    Msi,
    Dmg,
}

impl Bundle {
    pub fn tauri_name(self) -> &'static str {
        match self {
            Self::Deb => "deb",
            Self::Rpm => "rpm",
            Self::Appimage => "appimage",
            Self::Nsis => "nsis",
            Self::Msi => "msi",
            Self::Dmg => "dmg",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Deb => ".deb",
            Self::Rpm => ".rpm",
            Self::Appimage => ".AppImage",
            Self::Nsis => ".exe",
            Self::Msi => ".msi",
            Self::Dmg => ".dmg",
        }
    }

    pub fn for_host(linux_family: Option<&str>) -> Bundle {
        if cfg!(windows) {
            Bundle::Nsis
        } else if cfg!(target_os = "macos") {
            Bundle::Dmg
        } else {
            match linux_family {
                Some("debian") | Some("arch") => Bundle::Deb,
                Some("fedora") | Some("suse") => Bundle::Rpm,
                _ => Bundle::Appimage,
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Missing {
    pub name: String,
    pub why: String,
    pub install: String,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn quiet_std(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

fn which(program: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for ext in exts {
            let p = dir.join(format!("{program}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    if program == "cargo" || program == "rustc" {
        let bin = home()?.join(".cargo").join("bin");
        for ext in exts {
            let p = bin.join(format!("{program}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn find_tauri(root: &Path) -> Option<PathBuf> {
    let is_app = |d: &Path| d.join("src-tauri").join("tauri.conf.json").is_file();
    if is_app(root) {
        return Some(root.to_path_buf());
    }
    let skip = |n: &str| n.starts_with('.') || n == "node_modules" || n == "target";
    let mut level: Vec<PathBuf> = vec![root.to_path_buf()];
    for _ in 0..2 {
        let mut next = Vec::new();
        for d in &level {
            let Ok(rd) = std::fs::read_dir(d) else { continue };
            let mut subs: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir() && !p.file_name().and_then(|n| n.to_str()).is_some_and(skip))
                .collect();
            subs.sort();
            for s in subs {
                if is_app(&s) {
                    return Some(s);
                }
                next.push(s);
            }
        }
        level = next;
    }
    None
}

fn package_manager(root: &Path, app_dir: &Path) -> PackageManager {
    for dir in [root, app_dir] {
        let has = |f: &str| dir.join(f).exists();
        if has("pnpm-lock.yaml") || has("pnpm-workspace.yaml") {
            return PackageManager::Pnpm;
        }
        if has("bun.lockb") || has("bun.lock") {
            return PackageManager::Bun;
        }
        if has("yarn.lock") {
            return PackageManager::Yarn;
        }
        if has("package-lock.json") {
            return PackageManager::Npm;
        }
    }
    std::fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("packageManager").and_then(|p| p.as_str()).map(str::to_string))
        .map(|pm| match pm.split('@').next().unwrap_or("") {
            "pnpm" => PackageManager::Pnpm,
            "yarn" => PackageManager::Yarn,
            "bun" => PackageManager::Bun,
            _ => PackageManager::Npm,
        })
        .unwrap_or(PackageManager::Npm)
}

pub fn detect(root: &Path) -> Result<Project> {
    let app_dir = find_tauri(root).ok_or_else(|| {
        Error::hint(
            "The launcher can only build Tauri apps from source so far, and this isn't one.",
            "Use one of the downloads instead.",
        )
    })?;
    let conf: serde_json::Value = std::fs::read_to_string(app_dir.join("src-tauri/tauri.conf.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    Ok(Project {
        pm: package_manager(root, &app_dir),
        product_name: conf.get("productName").and_then(|v| v.as_str()).map(str::to_string),
        version: conf.get("version").and_then(|v| v.as_str()).map(str::to_string),
        root: root.to_path_buf(),
        app_dir,
    })
}

fn linux_install(family: Option<&str>, arch: &str, debian: &str, fedora: &str) -> String {
    match family {
        Some("debian") => format!("sudo apt install {debian}"),
        Some("fedora") => format!("sudo dnf install {fedora}"),
        Some("suse") => format!("sudo zypper install {fedora}"),
        _ => format!("sudo pacman -S --needed {arch}"),
    }
}

fn pkg_config_has(name: &str) -> bool {
    quiet_std(&mut std::process::Command::new("pkg-config"))
        .args(["--exists", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn requirements(project: Option<&Project>, linux_family: Option<&str>) -> Vec<Missing> {
    let mut out = Vec::new();
    let fam = linux_family;
    let mut need = |ok: bool, name: &str, why: &str, install: String| {
        if !ok {
            out.push(Missing { name: name.into(), why: why.into(), install });
        }
    };
    let os_install = |linux: String, other: &str| if cfg!(target_os = "linux") { linux } else { other.to_string() };

    need(
        which("git").is_some(),
        "git",
        "to download the source",
        os_install(linux_install(fam, "git", "git", "git"), if cfg!(windows) { "winget install Git.Git" } else { "xcode-select --install" }),
    );
    need(
        which("cargo").is_some(),
        "Rust (cargo)",
        "to compile the app",
        if cfg!(windows) { "winget install Rustlang.Rustup".into() } else { "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   (or see https://rustup.rs)".into() },
    );
    need(
        which("node").is_some(),
        "Node.js",
        "to build the interface",
        os_install(linux_install(fam, "nodejs npm", "nodejs npm", "nodejs npm"), if cfg!(windows) { "winget install OpenJS.NodeJS.LTS" } else { "brew install node" }),
    );
    if let Some(p) = project {
        let pm = p.pm.program();
        if p.pm != PackageManager::Npm {
            need(
                which(pm).is_some(),
                pm,
                "the project's package manager",
                match p.pm {
                    PackageManager::Pnpm => "npm install -g pnpm   (or: corepack enable pnpm)".into(),
                    PackageManager::Yarn => "npm install -g yarn   (or: corepack enable yarn)".into(),
                    PackageManager::Bun => "npm install -g bun".into(),
                    PackageManager::Npm => unreachable!(),
                },
            );
        }
    }
    if cfg!(windows) {
        need(
            msvc_installed(),
            "C++ Build Tools (MSVC)",
            "Rust links Windows programs with them",
            "winget install Microsoft.VisualStudio.2022.BuildTools --override \"--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended\"".into(),
        );
    }
    if cfg!(target_os = "macos") {
        need(
            quiet_std(&mut std::process::Command::new("xcode-select")).arg("-p").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()),
            "Xcode command line tools",
            "the compiler and linker",
            "xcode-select --install".into(),
        );
    }
    if cfg!(target_os = "linux") {
        let pc = which("pkg-config").is_some();
        need(pc, "pkg-config", "to find system libraries", linux_install(fam, "pkgconf", "pkg-config", "pkgconf-pkg-config"));
        if pc {
            need(
                pkg_config_has("webkit2gtk-4.1"),
                "WebKitGTK 4.1 (dev)",
                "the web view Tauri apps use",
                linux_install(fam, "webkit2gtk-4.1", "libwebkit2gtk-4.1-dev", "webkit2gtk4.1-devel"),
            );
            need(pkg_config_has("gtk+-3.0"), "GTK 3 (dev)", "window toolkit", linux_install(fam, "gtk3", "libgtk-3-dev", "gtk3-devel"));
            need(pkg_config_has("librsvg-2.0"), "librsvg (dev)", "icons", linux_install(fam, "librsvg", "librsvg2-dev", "librsvg2-devel"));
        }
        need(
            which("cc").is_some() || which("gcc").is_some() || which("clang").is_some(),
            "C compiler",
            "some Rust crates need one",
            linux_install(fam, "base-devel", "build-essential", "gcc"),
        );
    }
    out
}

fn msvc_installed() -> bool {
    let pf = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
    let vswhere = pf.join(r"Microsoft Visual Studio\Installer\vswhere.exe");
    if !vswhere.is_file() {
        return false;
    }
    quiet_std(&mut std::process::Command::new(vswhere))
        .args(["-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"])
        .output()
        .is_ok_and(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
}

async fn run(program: &str, args: &[&str], dir: &Path, env: &[(&str, String)], log: &Log, cancel: &AtomicBool) -> Result<()> {
    let exe = which(program).ok_or_else(|| Error::new(format!("{program} is not installed.")))?;
    let _ = log.send(format!("$ {program} {}", args.join(" ")));
    let mut cmd = Command::new(exe);
    quiet(&mut cmd)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("CI", "true")
        .env("NO_COLOR", "1")
        .env("CARGO_TERM_COLOR", "never");
    if let Some(home) = home() {
        let mut paths: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        paths.push(home.join(".cargo").join("bin"));
        if let Ok(p) = std::env::join_paths(paths) {
            cmd.env("PATH", p);
        }
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().map_err(|e| Error::new(format!("Couldn't start {program}: {e}")))?;

    let forward = |r: Box<dyn tokio::io::AsyncRead + Unpin + Send>| {
        let log = log.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(r).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let l = l.rsplit('\r').next().unwrap_or("").to_string();
                if !l.trim().is_empty() {
                    let _ = log.send(l);
                }
            }
        })
    };
    let out = forward(Box::new(child.stdout.take().expect("piped")));
    let err = forward(Box::new(child.stderr.take().expect("piped")));

    let status = loop {
        tokio::select! {
            s = child.wait() => break s.map_err(|e| Error::new(e.to_string()))?,
            _ = tokio::time::sleep(Duration::from_millis(250)) => {
                if cancel.load(Ordering::SeqCst) {
                    let _ = child.kill().await;
                    return Err(Error::cancelled());
                }
            }
        }
    };
    let _ = out.await;
    let _ = err.await;
    if status.success() {
        Ok(())
    } else {
        Err(Error::new(format!("{program} {} failed ({status}).", args.first().unwrap_or(&""))))
    }
}

pub async fn fetch_source(owner: &str, repo: &str, version: Option<&str>, dest: &Path, log: &Log, cancel: &AtomicBool) -> Result<String> {
    let url = format!("https://github.com/{owner}/{repo}.git");
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::new(e.to_string()))?;
    }
    let env = [("GIT_TERMINAL_PROMPT", "0".to_string())];

    let mut tag = None;
    if let Some(v) = version {
        let _ = log.send(format!("Looking for the {v} release tag…"));
        let out = quiet(&mut Command::new(which("git").ok_or_else(|| Error::new("git is not installed."))?))
            .args(["ls-remote", "--tags", &url])
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .await
            .map_err(|e| Error::new(e.to_string()))?;
        let tags = String::from_utf8_lossy(&out.stdout);
        let names: Vec<&str> = tags.lines().filter_map(|l| l.split("refs/tags/").nth(1)).map(|t| t.trim_end_matches("^{}")).collect();
        tag = [format!("v{v}"), v.to_string(), format!("app-v{v}")].into_iter().find(|t| names.contains(&t.as_str()));
        if tag.is_none() {
            let _ = log.send(format!("No tag for {v}; building the newest source instead."));
        }
    }

    let fresh = !dest.join(".git").is_dir();
    if fresh {
        let mut args = vec!["clone", "--depth", "1", "--recurse-submodules", "--shallow-submodules"];
        if let Some(t) = &tag {
            args.extend(["--branch", t.as_str()]);
        }
        let d = dest.to_string_lossy().to_string();
        args.extend([url.as_str(), d.as_str()]);
        run("git", &args, dest.parent().unwrap_or(Path::new(".")), &env, log, cancel).await?;
    } else {
        let refspec = tag.clone().map(|t| format!("refs/tags/{t}:refs/tags/{t}"));
        let mut args = vec!["fetch", "--depth", "1", "--force", "origin"];
        if let Some(r) = &refspec {
            args.push(r.as_str());
        }
        run("git", &args, dest, &env, log, cancel).await?;
        let target = tag.as_deref().map(|t| format!("refs/tags/{t}")).unwrap_or_else(|| "FETCH_HEAD".into());
        run("git", &["reset", "--hard", &target], dest, &env, log, cancel).await?;
        run("git", &["clean", "-fdx", "-e", "node_modules"], dest, &env, log, cancel).await?;
        run("git", &["submodule", "update", "--init", "--recursive", "--depth", "1"], dest, &env, log, cancel).await?;
    }
    Ok(tag.unwrap_or_else(|| "newest source".into()))
}

pub async fn build(project: &Project, bundle: Bundle, target_dir: &Path, log: &Log, cancel: Arc<AtomicBool>) -> Result<Vec<PathBuf>> {
    let pm = project.pm;
    let _ = log.send(format!("Installing dependencies with {}…", pm.program()));
    if let Err(e) = run(pm.program(), &pm.install_args(true), &project.root, &[], log, &cancel).await {
        if e.is_cancelled() {
            return Err(e);
        }
        let _ = log.send("Retrying without the frozen lockfile…".into());
        run(pm.program(), &pm.install_args(false), &project.root, &[], log, &cancel).await?;
    }

    let bundle_dir = target_dir.join("release").join("bundle").join(bundle.tauri_name());
    let _ = std::fs::remove_dir_all(&bundle_dir);
    std::fs::create_dir_all(target_dir).map_err(|e| Error::new(e.to_string()))?;
    let overrides = target_dir.join("zlauncher-build.conf.json");
    std::fs::write(&overrides, r#"{"bundle":{"createUpdaterArtifacts":false}}"#).map_err(|e| Error::new(e.to_string()))?;
    let overrides = overrides.to_string_lossy().to_string();
    let (prog, mut args) = pm.tauri();
    args.extend(["build", "--bundles", bundle.tauri_name(), "--config", overrides.as_str()]);
    let _ = log.send(format!("Building the {} (this takes a few minutes the first time)…", bundle.tauri_name()));
    run(prog, &args, &project.app_dir, &[("CARGO_TARGET_DIR", target_dir.to_string_lossy().to_string())], log, &cancel).await?;

    let files: Vec<PathBuf> = std::fs::read_dir(&bundle_dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.to_string_lossy().ends_with(bundle.extension()))
                .collect()
        })
        .unwrap_or_default();
    if files.is_empty() {
        return Err(Error::new(format!("The build finished but made no {} file.", bundle.extension())));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_tauri_layouts() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        std::fs::create_dir_all(root.join("apps/desktop/src-tauri")).unwrap();
        std::fs::write(root.join("apps/desktop/src-tauri/tauri.conf.json"), r#"{"productName":"ZDemo","version":"1.2.3"}"#).unwrap();
        std::fs::write(root.join("pnpm-lock.yaml"), "").unwrap();
        let p = detect(root).unwrap();
        assert_eq!(p.app_dir, root.join("apps/desktop"));
        assert_eq!(p.pm, PackageManager::Pnpm);
        assert_eq!(p.product_name.as_deref(), Some("ZDemo"));
        assert_eq!(p.version.as_deref(), Some("1.2.3"));

        let t2 = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t2.path().join("src-tauri")).unwrap();
        std::fs::write(t2.path().join("src-tauri/tauri.conf.json"), "{}").unwrap();
        std::fs::write(t2.path().join("package-lock.json"), "{}").unwrap();
        assert_eq!(detect(t2.path()).unwrap().pm, PackageManager::Npm);

        let t3 = tempfile::tempdir().unwrap();
        assert!(detect(t3.path()).is_err());
    }

    #[test]
    fn bundle_per_family() {
        if cfg!(target_os = "linux") {
            assert_eq!(Bundle::for_host(Some("arch")), Bundle::Deb);
            assert_eq!(Bundle::for_host(Some("fedora")), Bundle::Rpm);
            assert_eq!(Bundle::for_host(Some("unknown")), Bundle::Appimage);
        }
    }
}
