use std::collections::HashMap;

use tauri::AppHandle;

use crate::app::Shared;

const NOTIFIED_KEY: &str = "notified_updates";

fn show(app: &AppHandle, title: String, body: String) {
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        std::thread::spawn(move || {
            let r = notify_rust::Notification::new()
                .appname("ZLauncher")
                .icon("dev.theholyonez.zlauncher")
                .summary(&title)
                .body(&body)
                .show();
            if let Err(e) = r {
                tracing::debug!(%e, "could not show notification");
            }
        });
    }
    #[cfg(not(target_os = "linux"))]
    {
        use tauri_plugin_notification::NotificationExt;
        let app = app.clone();
        std::thread::spawn(move || {
            if let Err(e) = app.notification().builder().title(title).body(body).show() {
                tracing::debug!(%e, "could not show notification");
            }
        });
    }
}

pub fn updates(app: &AppHandle, state: &Shared) {
    if !state.settings().notify_updates {
        return;
    }
    let mut told: HashMap<String, String> = state.store.setting(NOTIFIED_KEY).ok().flatten().unwrap_or_default();
    let fresh: Vec<(String, String, String)> = state
        .library()
        .projects
        .into_iter()
        .filter_map(|p| {
            let new = p.installed.as_ref().filter(|i| i.present)?.update.clone()?;
            (told.get(&p.project.id) != Some(&new)).then_some((p.project.id, p.project.title, new))
        })
        .collect();
    if fresh.is_empty() {
        return;
    }
    let body = match fresh.as_slice() {
        [(_, title, v)] => format!("{title} {v} is ready to install."),
        many => {
            let names: Vec<&str> = many.iter().take(3).map(|(_, t, _)| t.as_str()).collect();
            let more = many.len().saturating_sub(3);
            format!("{}{} have updates.", names.join(", "), if more > 0 { format!(" and {more} more") } else { String::new() })
        }
    };
    show(app, "Updates available".into(), body);
    for (id, _, v) in fresh {
        told.insert(id, v);
    }
    let _ = state.store.set_setting(NOTIFIED_KEY, &told);
}
