#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod build;
mod github;
mod notify;
mod self_update;

use std::sync::Arc;
use std::time::Duration;

use app::{JobView, Library, Settings, Shared};
use tauri::{Manager, State};
use tauri_plugin_opener::OpenerExt;

type Res<T> = Result<T, String>;

async fn off_main<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Res<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn log(message: String) {
    tracing::warn!("ui: {message}");
    #[cfg(debug_assertions)]
    if message.starts_with("perf") {
        if let Some(dirs) = directories::ProjectDirs::from("dev", "theholyonez", "zlauncher") {
            use std::io::Write;
            let path = dirs.cache_dir().join("ui-perf.log");
            let _ = std::fs::create_dir_all(dirs.cache_dir());
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let tag = std::env::var("ZL_PERF_TAG").unwrap_or_default();
                let _ = writeln!(f, "{} {tag} {message}", zl_store::now());
            }
        }
    }
}

#[tauri::command]
async fn library(state: State<'_, Shared>) -> Res<Library> {
    let s = state.inner().clone();
    off_main(move || {
        let t = std::time::Instant::now();
        let l = s.library();
        tracing::debug!("library: {} projects in {:?}", l.projects.len(), t.elapsed());
        l
    })
    .await
}

#[tauri::command]
fn jobs(state: State<'_, Shared>) -> Vec<JobView> {
    state.jobs()
}

#[tauri::command]
fn refresh(app: tauri::AppHandle, state: State<'_, Shared>, force: bool) {
    let state = state.inner().clone();
    tauri::async_runtime::spawn(app::refresh(app, state, force));
}

#[tauri::command]
async fn install(app: tauri::AppHandle, state: State<'_, Shared>, project_id: String, url: Option<String>) -> Res<()> {
    let state = state.inner().clone();
    off_main(move || app::start_install(app, state, project_id, url)).await?
}

#[tauri::command]
async fn uninstall(app: tauri::AppHandle, state: State<'_, Shared>, project_id: String) -> Res<()> {
    let state = state.inner().clone();
    off_main(move || app::start_uninstall(app, state, project_id)).await?
}

#[tauri::command]
fn cancel(state: State<'_, Shared>, project_id: String) {
    state.cancel(&project_id);
}

#[tauri::command]
fn clear_job(state: State<'_, Shared>, project_id: String) {
    state.clear_job(&project_id);
}

#[tauri::command]
async fn launch(state: State<'_, Shared>, project_id: String) -> Res<()> {
    let state = state.inner().clone();
    off_main(move || launch_now(&state, &project_id)).await?
}

fn launch_now(state: &Shared, project_id: &str) -> Res<()> {
    if let Some(i) = state.store.install(project_id).map_err(|e| e.to_string())? {
        return zl_install::launch(&i.method, &i.handle).map_err(|e| e.message);
    }
    let found = state.external.lock().unwrap().get(project_id).cloned().ok_or("Not installed.")?;
    let title = state.project(project_id).map(|p| p.title).unwrap_or_default();
    zl_install::detect::launch(&found, &title).map_err(|e| e.message)
}

#[tauri::command]
async fn set_desktop_shortcut(app: tauri::AppHandle, state: State<'_, Shared>, project_id: String, want: bool) -> Res<()> {
    let state = state.inner().clone();
    off_main(move || shortcut_now(&app, &state, &project_id, want)).await?
}

fn shortcut_now(app: &tauri::AppHandle, state: &Shared, project_id: &str, want: bool) -> Res<()> {
    let mut i = state.store.install(project_id).map_err(|e| e.to_string())?.ok_or("Not installed.")?;
    i.handle = zl_install::set_desktop_shortcut(&i.method, &i.handle, want).map_err(|e| e.message)?;
    state.store.save_install(&i).map_err(|e| e.to_string())?;
    let _ = tauri::Emitter::emit(app, "library", ());
    Ok(())
}

#[tauri::command]
async fn build(app: tauri::AppHandle, state: State<'_, Shared>, project_id: String) -> Res<()> {
    let state = state.inner().clone();
    off_main(move || build::start(app, state, project_id)).await?
}

#[tauri::command]
async fn build_requirements(state: State<'_, Shared>) -> Res<Vec<zl_build::Missing>> {
    let s = state.inner().clone();
    off_main(move || build::requirements(&s)).await
}

#[tauri::command]
async fn build_cache_size(state: State<'_, Shared>) -> Res<u64> {
    let s = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || build::cache_size(&s)).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn clear_build_cache(state: State<'_, Shared>) -> Res<()> {
    let s = state.inner().clone();
    off_main(move || build::clear_cache(&s)).await?
}

#[tauri::command]
async fn github_status(state: State<'_, Shared>) -> Res<github::Status> {
    let token = state.github_token.read().map_err(|e| e.to_string())?.clone();
    let login = match token {
        Some(t) => github::whoami(&state.client, &t).await.ok(),
        None => None,
    };
    Ok(github::Status { login, device_flow: github::CLIENT_ID.is_some() })
}

fn keep_token(state: &Shared, token: Option<String>) -> Res<()> {
    match &token {
        Some(t) => state.store.set_setting(github::TOKEN_KEY, t).map_err(|e| e.to_string())?,
        None => state.store.set_setting(github::TOKEN_KEY, &Option::<String>::None).map_err(|e| e.to_string())?,
    }
    *state.github_token.write().map_err(|e| e.to_string())? = token;
    Ok(())
}

#[tauri::command]
async fn github_set_token(state: State<'_, Shared>, token: String) -> Res<String> {
    let token = token.trim().to_string();
    let login = github::whoami(&state.client, &token).await?;
    keep_token(state.inner(), Some(token))?;
    Ok(login)
}

#[tauri::command]
async fn github_sign_out(state: State<'_, Shared>) -> Res<()> {
    let s = state.inner().clone();
    off_main(move || keep_token(&s, None)).await?
}

#[tauri::command]
async fn github_device_start(state: State<'_, Shared>) -> Res<github::DeviceCode> {
    github::device_start(&state.client).await
}

#[tauri::command]
async fn github_device_poll(state: State<'_, Shared>, device_code: String, interval: u64) -> Res<serde_json::Value> {
    match github::device_poll(&state.client, &device_code, interval).await? {
        github::Poll::Done { token } => {
            let login = github::whoami(&state.client, &token).await?;
            keep_token(state.inner(), Some(token))?;
            Ok(serde_json::json!({ "status": "done", "login": login }))
        }
        other => serde_json::to_value(other).map_err(|e| e.to_string()),
    }
}

#[tauri::command]
async fn admin_status() -> Res<zl_install::AdminStatus> {
    off_main(zl_install::admin_status).await
}

#[tauri::command]
async fn admin_unlock(password: String) -> Res<()> {
    off_main(move || zl_install::admin_unlock(&password).map_err(|e| e.message)).await?
}

#[tauri::command]
async fn admin_lock() -> Res<()> {
    off_main(zl_install::admin_lock).await
}

#[tauri::command]
async fn forget(app: tauri::AppHandle, state: State<'_, Shared>, project_id: String) -> Res<()> {
    let state = state.inner().clone();
    off_main(move || {
        state.store.remove_install(&project_id).map_err(|e| e.to_string())?;
        let _ = tauri::Emitter::emit(&app, "library", ());
        Ok(())
    })
    .await?
}

#[tauri::command]
async fn settings(state: State<'_, Shared>) -> Res<Settings> {
    let s = state.inner().clone();
    off_main(move || s.settings()).await
}

#[tauri::command]
async fn set_settings(app: tauri::AppHandle, state: State<'_, Shared>, settings: Settings) -> Res<()> {
    let state = state.inner().clone();
    off_main(move || {
        let before = state.settings();
        state.set_settings(&settings)?;
        if before.linux_format != settings.linux_format {
            let _ = tauri::Emitter::emit(&app, "library", ());
        }
        Ok(())
    })
    .await?
}

#[tauri::command]
async fn clear_cache(app: tauri::AppHandle, state: State<'_, Shared>) -> Res<()> {
    let st = state.inner().clone();
    let s2 = st.clone();
    off_main(move || s2.store.clear_crawls().map_err(|e| e.to_string())).await??;
    tauri::async_runtime::spawn(app::refresh(app, st, true));
    Ok(())
}

#[tauri::command]
fn self_update_status(app: tauri::AppHandle) -> self_update::View {
    self_update::view(&app)
}

#[tauri::command]
async fn self_update_check(app: tauri::AppHandle, state: State<'_, Shared>) -> Res<()> {
    let auto = state.settings().auto_update;
    self_update::check(app, auto).await;
    Ok(())
}

#[tauri::command]
async fn self_update_install(app: tauri::AppHandle) -> Res<()> {
    self_update::install(app).await
}

#[tauri::command]
fn open_url(app: tauri::AppHandle, url: String) -> Res<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Only web links can be opened.".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn reveal(app: tauri::AppHandle, path: String) -> Res<()> {
    app.opener().reveal_item_in_dir(path).map_err(|e| e.to_string())
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,zlauncher=info".into()))
        .init();

    let mut builder = tauri::Builder::default();
    let single = !(cfg!(debug_assertions) && std::env::var("ZL_DEV_MULTI").as_deref() == Ok("1"));
    if single {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }));
    }
    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(self_update::SelfUpdate::default())
        .setup(|app| {
            let dirs = directories::ProjectDirs::from("dev", "theholyonez", "zlauncher")
                .ok_or("no home folder to keep the launcher's data in")?;
            let state: Shared = Arc::new(app::App::new(dirs.data_dir().to_path_buf(), dirs.cache_dir().to_path_buf())?);
            app.manage(state.clone());

            #[cfg(debug_assertions)]
            if let Ok(hash) = std::env::var("ZL_DEV_HASH") {
                if let Some(w) = app.get_webview_window("main") {
                    let hash = serde_json::to_string(&hash).unwrap_or_default();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs(3));
                        let _ = w.eval(format!("location.hash = {hash}; location.reload();"));
                    });
                }
            }

            tauri::async_runtime::spawn(async {
                loop {
                    tokio::time::sleep(Duration::from_secs(4 * 60)).await;
                    let _ = tauri::async_runtime::spawn_blocking(zl_install::admin_keep_alive).await;
                }
            });

            let (handle, st) = (app.handle().clone(), state.clone());
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(8)).await;
                loop {
                    let auto = st.settings().auto_update;
                    self_update::check(handle.clone(), auto).await;
                    tokio::time::sleep(Duration::from_secs(6 * 60 * 60)).await;
                }
            });

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    app::refresh(handle.clone(), state.clone(), false).await;
                    tokio::time::sleep(Duration::from_secs(30 * 60)).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            log,
            self_update_status,
            self_update_check,
            self_update_install,
            library,
            jobs,
            refresh,
            install,
            uninstall,
            cancel,
            clear_job,
            launch,
            set_desktop_shortcut,
            forget,
            build,
            build_requirements,
            build_cache_size,
            clear_build_cache,
            github_status,
            github_set_token,
            github_sign_out,
            github_device_start,
            github_device_poll,
            settings,
            set_settings,
            clear_cache,
            admin_status,
            admin_unlock,
            admin_lock,
            open_url,
            reveal,
        ])
        .build(tauri::generate_context!())
        .expect("error while building ZLauncher")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<Shared>() {
                    state.flush_settings();
                }
                zl_install::admin_lock();
            }
        });
}
