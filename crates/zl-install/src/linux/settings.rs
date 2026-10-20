use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Paper,
    Blueprint,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PrivilegeMode {
    #[default]
    Session,

    Each,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub appimage_dir: PathBuf,
    pub desktop_shortcut_default: bool,
    pub remove_original_appimage: bool,
    pub remove_original_package: bool,
    pub sound_enabled: bool,
    pub sound_volume: f32,
    pub theme: Theme,
    pub system_frame: bool,

    pub update_dir: Option<PathBuf>,
    pub check_updates_on_start: bool,

    pub auto_apply_updates: bool,
    pub privilege_mode: PrivilegeMode,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            appimage_dir: dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/"))
                .join("Applications"),
            desktop_shortcut_default: false,
            remove_original_appimage: true,
            remove_original_package: false,
            sound_enabled: true,
            sound_volume: 0.25,
            theme: Theme::System,
            system_frame: false,
            update_dir: None,
            check_updates_on_start: true,
            auto_apply_updates: false,
            privilege_mode: PrivilegeMode::Session,
        }
    }
}

pub fn settings_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("zlauncher")
        .join("settings.json")
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read_to_string(settings_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = settings_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }
}
