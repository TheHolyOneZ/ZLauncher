use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::linux::distro::{Family, ToolStatus};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum PackageKind {
    Deb,
    Rpm,
    AppImage,
}

impl PackageKind {
    pub fn label(self) -> &'static str {
        match self {
            PackageKind::Deb => ".deb",
            PackageKind::Rpm => ".rpm",
            PackageKind::AppImage => "AppImage",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DesktopEntry {
    pub path: String,
    pub name: String,
    pub exec: Option<String>,
    pub icon: Option<String>,
    pub comment: Option<String>,
    pub categories: Vec<String>,
    pub no_display: bool,

    pub raw: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RawDependency {
    pub name: String,
    pub version_req: Option<String>,

    pub alternatives: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PackageModel {
    pub kind: PackageKind,
    pub source_path: PathBuf,
    pub file_size: u64,

    pub name: String,

    pub display_name: String,
    pub version: String,
    pub arch: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub maintainer: Option<String>,
    pub installed_size: Option<u64>,
    pub dependencies: Vec<RawDependency>,
    pub desktop_entries: Vec<DesktopEntry>,

    pub icon_data_url: Option<String>,
    pub file_count: usize,

    pub top_level_paths: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NativeFormat {
    Pacman,
    Deb,
    Rpm,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Strategy {
    NativeInstall { format: NativeFormat },

    ConvertToNative { target: NativeFormat },

    AppImageIntegrate,
    Unsupported { reason: String },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum StageId {
    Inspect,
    Translate,
    Build,
    Install,
    Move,
    Shortcuts,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Stage {
    pub id: StageId,
    pub label: String,
}

impl Stage {
    pub fn new(id: StageId) -> Self {
        let label = match id {
            StageId::Inspect => "Inspect",
            StageId::Translate => "Translate",
            StageId::Build => "Build",
            StageId::Install => "Install",
            StageId::Move => "Move",
            StageId::Shortcuts => "Shortcuts",
        };
        Stage {
            id,
            label: label.to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum DepStatus {
    Found,
    Unresolved,
    Skipped { reason: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DepResolution {
    pub raw: String,
    pub resolved: Option<String>,
    #[serde(flatten)]
    pub status: DepStatus,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstallPlan {
    pub package: PackageModel,
    pub host: Family,
    pub strategy: Strategy,
    pub stages: Vec<Stage>,
    pub dependencies: Vec<DepResolution>,
    pub needs_root: bool,
    pub missing_tools: Vec<ToolStatus>,
    pub warnings: Vec<String>,

    #[serde(default)]
    pub update_of: Option<crate::linux::update::UpdateMatch>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstallOptions {
    pub menu_entry: bool,
    pub desktop_shortcut: bool,
    pub remove_original: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InstallKind {
    Native,
    AppImage,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledEntry {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub version: String,
    pub source_kind: PackageKind,
    pub install_kind: InstallKind,

    pub host_package: Option<String>,
    pub appimage_path: Option<PathBuf>,

    pub desktop_files: Vec<PathBuf>,

    #[serde(default)]
    pub launcher: Option<PathBuf>,
    pub icon_path: Option<PathBuf>,
    pub original_path: PathBuf,

    pub installed_at: String,
    pub unresolved_deps: Vec<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub maintainer: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub file_size: Option<u64>,

    #[serde(default)]
    pub update_dir: Option<PathBuf>,

    #[serde(default)]
    pub ignored_versions: Vec<String>,

    #[serde(default)]
    pub history: Vec<VersionRecord>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VersionRecord {
    pub version: String,
    pub installed_at: String,
    pub source_kind: PackageKind,
}

impl InstalledEntry {
    pub fn with_metadata(mut self, model: &PackageModel) -> Self {
        self.summary = model.summary.clone();
        self.description = model.description.clone();
        self.maintainer = model.maintainer.clone();
        self.homepage = model.homepage.clone();
        self.license = model.license.clone();
        self.arch = Some(model.arch.clone());
        self.file_size = Some(model.file_size);
        self
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StageStatus {
    Pending,
    Active,
    Done,
    Failed,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
#[allow(clippy::large_enum_variant)]
pub enum JobEvent {
    Stage {
        index: usize,
        status: StageStatus,
    },

    Narrate {
        text: String,
    },

    Log {
        line: String,
    },

    NeedsAuth,
    Done {
        entry: InstalledEntry,
    },
    Failed {
        message: String,
        hint: Option<String>,
    },
}
