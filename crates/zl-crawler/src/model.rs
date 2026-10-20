use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Windows,
    Linux,
    Macos,
    Android,
    Ios,
    Browser,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    X64,
    Arm64,
    Universal,
}

impl Arch {
    pub fn current() -> Arch {
        if cfg!(target_arch = "aarch64") {
            Arch::Arm64
        } else {
            Arch::X64
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Deb,
    Rpm,
    AppImage,
    Pacman,
    Msi,
    Setup,
    PortableExe,
    Dmg,
    AppArchive,
    Zip,
    TarGz,
    Binary,
    Apk,
    Ipa,
    Extension,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Deb => ".deb",
            Kind::Rpm => ".rpm",
            Kind::AppImage => "AppImage",
            Kind::Pacman => "Arch package",
            Kind::Msi => ".msi",
            Kind::Setup => "Installer",
            Kind::PortableExe => "Portable .exe",
            Kind::Dmg => ".dmg",
            Kind::AppArchive => ".app archive",
            Kind::Zip => ".zip",
            Kind::TarGz => ".tar.gz",
            Kind::Binary => "Binary",
            Kind::Apk => ".apk",
            Kind::Ipa => ".ipa",
            Kind::Extension => "Extension",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    Manifest,
    Alias,
    Link,
    Script,
    Checksums,
    Listing,
    Github,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Artifact {
    pub project_id: String,
    pub url: String,
    pub filename: String,
    pub os: Os,
    pub arch: Option<Arch>,
    pub kind: Kind,
    pub version: Option<String>,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    pub source: Source,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CrawlReport {
    pub project_id: String,
    pub artifacts: Vec<Artifact>,
    pub version: Option<String>,
    pub notes: Vec<String>,
    #[serde(default)]
    pub release_notes: Option<String>,
    #[serde(default)]
    pub released: Option<String>,
}
