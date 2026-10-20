use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde_json::json;
use tokio::sync::mpsc;

use crate::linux::{
    self, convert::depmap::HostVerifier, job::JobContext, manage, model::InstallKind, model::JobEvent,
    model::Strategy, registry::Registry, settings::Settings, InstallOptions,
};
use crate::{step, Error, Event, Events, Installed, Request, Result};

fn entry_id(handle: &serde_json::Value) -> Result<String> {
    handle
        .get("entry")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| Error::new("This install record is missing its Linux entry id."))
}

fn entry(handle: &serde_json::Value) -> Result<linux::InstalledEntry> {
    let id = entry_id(handle)?;
    Registry::load()
        .find(&id)
        .cloned()
        .ok_or_else(|| Error::new("The launcher no longer knows where this was installed."))
}

async fn pump(mut rx: mpsc::UnboundedReceiver<JobEvent>, events: &Events) -> Option<std::result::Result<linux::InstalledEntry, Error>> {
    while let Some(ev) = rx.recv().await {
        match ev {
            JobEvent::Narrate { text } => step(events, text),
            JobEvent::Log { line } => {
                let _ = events.send(Event::Log { line });
            }
            JobEvent::NeedsAuth => {
                let _ = events.send(Event::NeedsAuth);
            }
            JobEvent::Stage { .. } => {}
            JobEvent::Done { entry } => return Some(Ok(entry)),
            JobEvent::Failed { message, hint } => return Some(Err(Error { message, hint })),
        }
    }
    None
}

pub async fn install(req: Request<'_>, events: Events, cancel: Arc<AtomicBool>) -> Result<Installed> {
    step(&events, "Reading the package");
    let file = req.file.to_path_buf();
    let model = tokio::task::spawn_blocking(move || linux::inspect(&file))
        .await
        .map_err(|e| Error::new(e.to_string()))?
        .map_err(|e| Error::new(format!("Couldn't read the package: {e}")))?;

    let system = linux::detect_system();
    let verifier = HostVerifier::new(system.family);
    let plan = linux::plan(&model, &system, &verifier);
    if let Strategy::Unsupported { reason } = &plan.strategy {
        return Err(Error::new(reason.clone()));
    }
    if !plan.missing_tools.is_empty() {
        let names: Vec<String> = plan.missing_tools.iter().map(|t| t.name.clone()).collect();
        return Err(Error::hint(
            format!("This install needs tools that are missing: {}.", names.join(", ")),
            "Install them with your package manager, or choose the AppImage instead.",
        ));
    }
    for w in &plan.warnings {
        let _ = events.send(Event::Log { line: format!("note: {w}") });
    }
    let method = match plan.strategy {
        Strategy::AppImageIntegrate => "appimage",
        Strategy::NativeInstall { .. } => "native",
        Strategy::ConvertToNative { .. } => "converted",
        Strategy::Unsupported { .. } => unreachable!(),
    };

    let options = InstallOptions {
        menu_entry: req.options.menu_entry,
        desktop_shortcut: req.options.desktop_shortcut,
        remove_original: false,
    };
    let mut settings = Settings::load();
    settings.remove_original_appimage = false;
    settings.remove_original_package = false;

    let (tx, rx) = mpsc::unbounded_channel();
    let ctx = JobContext::new(plan, options, system, settings, tx, cancel);
    tokio::spawn(linux::job::run(ctx));
    let entry = pump(rx, &events).await.unwrap_or_else(|| Err(Error::new("The install stopped without an answer.")))?;

    Ok(Installed { method: method.into(), handle: json!({ "entry": entry.id }), version: Some(entry.version) })
}

pub async fn uninstall(_method: &str, handle: &serde_json::Value, events: Events) -> Result<()> {
    let id = entry_id(handle)?;
    let system = linux::detect_system();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let forward = {
        let events = events.clone();
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                match ev {
                    JobEvent::Narrate { text } => step(&events, text),
                    JobEvent::Log { line } => {
                        let _ = events.send(Event::Log { line });
                    }
                    JobEvent::NeedsAuth => {
                        let _ = events.send(Event::NeedsAuth);
                    }
                    _ => {}
                }
            }
        })
    };
    let result = manage::uninstall(&id, &system, &tx).await;
    drop(tx);
    let _ = forward.await;
    result.map(|_| ()).map_err(|e| Error { message: e.message, hint: e.hint })
}

pub fn launch(_method: &str, handle: &serde_json::Value) -> Result<()> {
    manage::launch(&entry(handle)?).map_err(Error::new)
}

pub fn is_present(_method: &str, handle: &serde_json::Value) -> bool {
    let Ok(e) = entry(handle) else { return false };
    match e.install_kind {
        InstallKind::AppImage => e.appimage_path.as_ref().is_some_and(|p| p.exists()),
        InstallKind::Native => e.host_package.as_deref().is_none_or(package_installed),
    }
}

fn package_installed(name: &str) -> bool {
    let check = |prog: &str, args: &[&str]| {
        linux::distro::which(prog).map(|p| {
            std::process::Command::new(p)
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
    };
    check("pacman", &["-Q", name])
        .or_else(|| check("dpkg-query", &["-W", name]))
        .or_else(|| check("rpm", &["-q", name]))
        .unwrap_or(true)
}

pub fn has_desktop_shortcut(_method: &str, handle: &serde_json::Value) -> bool {
    entry(handle).map(|e| manage::has_desktop_shortcut(&e)).unwrap_or(false)
}

pub fn set_desktop_shortcut(_method: &str, handle: &serde_json::Value, want: bool) -> Result<serde_json::Value> {
    let id = entry_id(handle)?;
    manage::set_desktop_shortcut(&id, want).map_err(Error::new)?;
    Ok(handle.clone())
}
