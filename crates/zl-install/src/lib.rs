use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use zl_crawler::{Arch, Artifact, Os};

mod choose;
pub mod detect;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
mod linux_backend;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

pub use choose::{installable, recommend, LinuxFormat};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub os: Os,
    pub arch: Arch,
    pub linux_family: Option<String>,
    pub name: String,
    pub video: bool,
}

#[cfg(target_os = "linux")]
fn linux_video_ok() -> bool {
    let dirs: Vec<std::path::PathBuf> = std::env::var_os("GST_PLUGIN_SYSTEM_PATH_1_0")
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_else(|| {
            ["/usr/lib/gstreamer-1.0", "/usr/lib64/gstreamer-1.0", "/usr/lib/x86_64-linux-gnu/gstreamer-1.0", "/usr/lib/aarch64-linux-gnu/gstreamer-1.0"]
                .iter()
                .map(PathBuf::from)
                .collect()
        });
    let has = |name: &str| dirs.iter().any(|d| d.join(name).exists());
    has("libgstisomp4.so")
        && has("libgstautodetect.so")
        && ["libgstlibav.so", "libgstopenh264.so", "libgstva.so", "libgstvaapi.so"].iter().any(|n| has(n))
}

pub fn host() -> Host {
    #[cfg(target_os = "linux")]
    {
        let sys = linux::detect_system();
        Host {
            os: Os::Linux,
            arch: Arch::current(),
            linux_family: Some(format!("{:?}", sys.family).to_lowercase()),
            name: sys.pretty_name.clone(),
            video: linux_video_ok(),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        Host {
            os: Os::current(),
            arch: Arch::current(),
            linux_family: None,
            name: if cfg!(windows) { "Windows".into() } else { "macOS".into() },
            video: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    pub menu_entry: bool,
    pub desktop_shortcut: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { menu_entry: true, desktop_shortcut: false }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    Step { text: String },
    Log { line: String },
    NeedsAuth,
}

pub type Events = UnboundedSender<Event>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Installed {
    pub method: String,
    pub handle: serde_json::Value,
    pub version: Option<String>,
}

#[derive(Debug, Clone, thiserror::Error, Serialize)]
#[error("{message}")]
pub struct Error {
    pub message: String,
    pub hint: Option<String>,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), hint: None }
    }
    pub fn hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self { message: message.into(), hint: Some(hint.into()) }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::new(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn step(events: &Events, text: impl Into<String>) {
    let _ = events.send(Event::Step { text: text.into() });
}

pub fn apps_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("zlauncher").join("apps")
}

pub struct Request<'a> {
    pub file: &'a Path,
    pub artifact: &'a Artifact,
    pub title: &'a str,
    pub options: Options,
}

pub async fn install(req: Request<'_>, events: Events, cancel: Arc<AtomicBool>) -> Result<Installed> {
    if !installable(req.artifact.kind, Os::current()) {
        return Err(Error::new(format!(
            "{} files can't be installed on this system; the file was downloaded to {}.",
            req.artifact.kind.label(),
            req.file.display()
        )));
    }
    #[cfg(target_os = "linux")]
    return linux_backend::install(req, events, cancel).await;
    #[cfg(windows)]
    return windows::install(req, events, cancel).await;
    #[cfg(target_os = "macos")]
    return macos::install(req, events, cancel).await;
    #[allow(unreachable_code)]
    {
        let _ = (events, cancel);
        Err(Error::new("This system is not supported."))
    }
}

pub async fn uninstall(method: &str, handle: &serde_json::Value, events: Events) -> Result<()> {
    #[cfg(target_os = "linux")]
    return linux_backend::uninstall(method, handle, events).await;
    #[cfg(windows)]
    return windows::uninstall(method, handle, events).await;
    #[cfg(target_os = "macos")]
    return macos::uninstall(method, handle, events).await;
    #[allow(unreachable_code)]
    {
        let _ = (method, handle, events);
        Err(Error::new("This system is not supported."))
    }
}

pub fn launch(method: &str, handle: &serde_json::Value) -> Result<()> {
    #[cfg(target_os = "linux")]
    return linux_backend::launch(method, handle);
    #[cfg(windows)]
    return windows::launch(method, handle);
    #[cfg(target_os = "macos")]
    return macos::launch(method, handle);
    #[allow(unreachable_code)]
    {
        let _ = (method, handle);
        Err(Error::new("This system is not supported."))
    }
}

pub fn is_present(method: &str, handle: &serde_json::Value) -> bool {
    #[cfg(target_os = "linux")]
    return linux_backend::is_present(method, handle);
    #[cfg(windows)]
    return windows::is_present(method, handle);
    #[cfg(target_os = "macos")]
    return macos::is_present(method, handle);
    #[allow(unreachable_code)]
    {
        let _ = (method, handle);
        false
    }
}

pub fn has_desktop_shortcut(method: &str, handle: &serde_json::Value) -> bool {
    #[cfg(target_os = "linux")]
    return linux_backend::has_desktop_shortcut(method, handle);
    #[cfg(windows)]
    return windows::has_desktop_shortcut(method, handle);
    #[cfg(target_os = "macos")]
    return macos::has_desktop_shortcut(method, handle);
    #[allow(unreachable_code)]
    {
        let _ = (method, handle);
        false
    }
}

pub fn set_desktop_shortcut(method: &str, handle: &serde_json::Value, want: bool) -> Result<serde_json::Value> {
    #[cfg(target_os = "linux")]
    return linux_backend::set_desktop_shortcut(method, handle, want);
    #[cfg(windows)]
    return windows::set_desktop_shortcut(method, handle, want);
    #[cfg(target_os = "macos")]
    return macos::set_desktop_shortcut(method, handle, want);
    #[allow(unreachable_code)]
    {
        let _ = (method, handle, want);
        Err(Error::new("This system is not supported."))
    }
}

#[cfg(windows)]
pub(crate) fn safe_name(title: &str) -> String {
    title.chars().map(|c| if c.is_alphanumeric() || " -_.".contains(c) { c } else { '_' }).collect::<String>().trim().to_string()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminStatus {
    pub supported: bool,
    pub unlocked: bool,
}

pub fn admin_status() -> AdminStatus {
    #[cfg(target_os = "linux")]
    {
        let root = unsafe { libc::geteuid() } == 0;
        let supported = !root && linux::elevate::sudo_path().is_some();
        AdminStatus { supported, unlocked: supported && linux::elevate::session_unlocked() }
    }
    #[cfg(not(target_os = "linux"))]
    {
        AdminStatus { supported: false, unlocked: false }
    }
}

pub fn admin_unlock(password: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::elevate::session_unlock(password).map_err(|e| Error::new(e.to_string()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = password;
        Err(Error::new("Not needed on this system."))
    }
}

pub fn admin_keep_alive() -> bool {
    #[cfg(target_os = "linux")]
    return linux::elevate::session_refresh();
    #[cfg(not(target_os = "linux"))]
    false
}

pub fn admin_lock() {
    #[cfg(target_os = "linux")]
    linux::elevate::session_lock();
}
