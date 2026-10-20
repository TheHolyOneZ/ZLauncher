use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub state: &'static str,
    pub current: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub done: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
    pub can_install: bool,
    pub reason: Option<String>,
}

#[derive(Default)]
pub struct SelfUpdate {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    view: View,
    update: Option<Update>,
    bytes: Option<Vec<u8>>,
    busy: bool,
}

fn installable() -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("This is a development build.".into());
    }
    #[cfg(target_os = "linux")]
    {
        use tauri::utils::config::BundleType;
        let family = zl_install::host().linux_family.unwrap_or_default();
        return match tauri::utils::platform::bundle_type() {
            Some(BundleType::AppImage) => Ok(()),
            Some(BundleType::Deb) if family == "debian" => Ok(()),
            Some(BundleType::Rpm) if family == "fedora" || family == "suse" => Ok(()),
            Some(BundleType::Deb) | Some(BundleType::Rpm) => Err("This copy came from a package your system doesn't install directly.".into()),
            _ => Err("This copy wasn't installed from one of the launcher's own packages.".into()),
        };
    }
    #[allow(unreachable_code)]
    Ok(())
}

fn publish(app: &AppHandle, f: impl FnOnce(&mut Inner)) {
    let st = app.state::<SelfUpdate>();
    let view = {
        let mut g = st.inner.lock().unwrap();
        let before = g.view.state;
        f(&mut g);
        if g.view.state != before {
            tracing::info!(state = g.view.state, version = ?g.view.version, "launcher update");
        }
        g.view.clone()
    };
    let _ = app.emit("self-update", view);
}

pub fn view(app: &AppHandle) -> View {
    let st = app.state::<SelfUpdate>();
    let mut v = st.inner.lock().unwrap().view.clone();
    if v.state.is_empty() {
        v.state = "idle";
    }
    v.current = app.package_info().version.to_string();
    let ok = installable();
    v.can_install = ok.is_ok();
    v.reason = ok.err();
    v
}

pub async fn check(app: AppHandle, download: bool) {
    {
        let st = app.state::<SelfUpdate>();
        let mut g = st.inner.lock().unwrap();
        if g.busy || g.bytes.is_some() {
            return;
        }
        g.busy = true;
    }
    let current = app.package_info().version.to_string();
    let ok = installable();
    publish(&app, |g| {
        g.view = View { state: "checking", current: current.clone(), can_install: ok.is_ok(), reason: ok.clone().err(), ..Default::default() };
    });
    let found = match app.updater() {
        Ok(u) => u.check().await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    match found {
        Ok(Some(u)) => {
            let (version, notes) = (u.version.clone(), u.body.clone());
            publish(&app, |g| {
                g.update = Some(u);
                g.view.state = "available";
                g.view.version = Some(version);
                g.view.notes = notes;
                g.busy = false;
            });
            if download && ok.is_ok() {
                let _ = fetch(&app).await;
            }
        }
        Ok(None) => publish(&app, |g| {
            g.view.state = "latest";
            g.busy = false;
        }),
        Err(e) => {
            tracing::info!(%e, "update check failed");
            publish(&app, |g| {
                g.view.state = "error";
                g.view.error = Some(e);
                g.busy = false;
            });
        }
    }
}

async fn fetch(app: &AppHandle) -> Result<(), String> {
    let update = {
        let st = app.state::<SelfUpdate>();
        let mut g = st.inner.lock().unwrap();
        if g.busy {
            return Err("Already downloading.".into());
        }
        g.busy = true;
        g.update.clone().ok_or("No update was found.")?
    };
    publish(app, |g| {
        g.view.state = "downloading";
        g.view.done = 0;
        g.view.total = None;
    });
    let a = app.clone();
    let mut done = 0u64;
    let mut last = std::time::Instant::now();
    let r = update
        .download(
            |chunk, total| {
                done += chunk as u64;
                if last.elapsed().as_millis() >= 200 {
                    last = std::time::Instant::now();
                    publish(&a, |g| {
                        g.view.done = done;
                        g.view.total = total;
                    });
                }
            },
            || {},
        )
        .await;
    match r {
        Ok(bytes) => {
            publish(app, |g| {
                g.bytes = Some(bytes);
                g.view.state = "ready";
                g.busy = false;
            });
            Ok(())
        }
        Err(e) => {
            let e = e.to_string();
            publish(app, |g| {
                g.view.state = "error";
                g.view.error = Some(e.clone());
                g.busy = false;
            });
            Err(e)
        }
    }
}

pub async fn install(app: AppHandle) -> Result<(), String> {
    installable()?;
    let have = app.state::<SelfUpdate>().inner.lock().unwrap().bytes.is_some();
    if !have {
        fetch(&app).await?;
    }
    let (update, bytes) = {
        let st = app.state::<SelfUpdate>();
        let mut g = st.inner.lock().unwrap();
        (g.update.clone().ok_or("No update was found.")?, g.bytes.take().ok_or("The update isn't downloaded.")?)
    };
    publish(&app, |g| g.view.state = "installing");
    if let Some(s) = app.try_state::<crate::app::Shared>() {
        s.flush_settings();
    }
    let r = tauri::async_runtime::spawn_blocking(move || update.install(bytes)).await.map_err(|e| e.to_string())?;
    match r {
        Ok(()) => app.restart(),
        Err(e) => {
            let e = e.to_string();
            publish(&app, |g| {
                g.view.state = "error";
                g.view.error = Some(e.clone());
            });
            Err(e)
        }
    }
}
