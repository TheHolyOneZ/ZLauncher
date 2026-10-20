use serde::{Deserialize, Serialize};
use zl_crawler::{Arch, Artifact, Kind, Os};

use crate::Host;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LinuxFormat {
    #[default]
    Auto,
    Appimage,
}

pub fn installable(kind: Kind, os: Os) -> bool {
    match os {
        Os::Linux => matches!(kind, Kind::Deb | Kind::Rpm | Kind::AppImage),
        Os::Windows => matches!(kind, Kind::Setup | Kind::Msi | Kind::PortableExe),
        Os::Macos => matches!(kind, Kind::Dmg | Kind::AppArchive),
        _ => false,
    }
}

fn kind_order(host: &Host, pref: LinuxFormat) -> Vec<Kind> {
    match host.os {
        Os::Linux => {
            let native: &[Kind] = match host.linux_family.as_deref() {
                Some("debian") => &[Kind::Deb, Kind::Rpm],
                Some("fedora") | Some("suse") => &[Kind::Rpm, Kind::Deb],
                Some("arch") => &[Kind::Deb, Kind::Rpm],
                _ => &[],
            };
            match pref {
                LinuxFormat::Appimage => [&[Kind::AppImage][..], native].concat(),
                LinuxFormat::Auto => [native, &[Kind::AppImage][..]].concat(),
            }
        }
        Os::Windows => vec![Kind::Setup, Kind::Msi, Kind::PortableExe],
        Os::Macos => vec![Kind::Dmg, Kind::AppArchive],
        _ => Vec::new(),
    }
}

fn arch_score(a: Option<Arch>, host: Arch) -> Option<u8> {
    match a {
        Some(x) if x == host => Some(0),
        Some(Arch::Universal) => Some(1),
        None if host == Arch::X64 => Some(2),
        None => Some(3),
        Some(Arch::X64) if host == Arch::Arm64 && matches!(Os::current(), Os::Macos | Os::Windows) => Some(4),
        Some(_) => None,
    }
}

pub fn recommend<'a>(artifacts: &'a [Artifact], host: &Host, pref: LinuxFormat) -> Option<&'a Artifact> {
    let order = kind_order(host, pref);
    artifacts
        .iter()
        .filter(|a| a.os == host.os)
        .filter_map(|a| {
            let k = order.iter().position(|k| *k == a.kind)?;
            let s = arch_score(a.arch, host.arch)?;
            Some(((k, s), a))
        })
        .min_by_key(|(score, _)| *score)
        .map(|(_, a)| a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zl_crawler::Source;

    fn art(name: &str) -> Artifact {
        let c = zl_crawler::classify(name).unwrap();
        Artifact {
            project_id: "x".into(),
            url: format!("https://zsync.eu/x/releases/{name}"),
            filename: name.into(),
            os: c.os,
            arch: c.arch,
            kind: c.kind,
            version: c.version,
            size: None,
            sha256: None,
            source: Source::Link,
        }
    }

    fn host(os: Os, arch: Arch, family: Option<&str>) -> Host {
        Host { os, arch, linux_family: family.map(str::to_string), name: String::new(), video: false }
    }

    #[test]
    fn picks_per_system() {
        let all: Vec<Artifact> = [
            "Z_1.0.0_amd64.deb",
            "Z-1.0.0-1.x86_64.rpm",
            "Z_1.0.0_amd64.AppImage",
            "Z_1.0.0_x64_en-US.msi",
            "Z_1.0.0_x64-setup.exe",
            "Z_1.0.0_x64.dmg",
            "Z_1.0.0_aarch64.dmg",
        ]
        .into_iter()
        .map(art)
        .collect();
        let pick = |h: Host, p| recommend(&all, &h, p).map(|a| a.filename.clone());
        assert_eq!(pick(host(Os::Linux, Arch::X64, Some("arch")), LinuxFormat::Auto).unwrap(), "Z_1.0.0_amd64.deb");
        assert_eq!(pick(host(Os::Linux, Arch::X64, Some("fedora")), LinuxFormat::Auto).unwrap(), "Z-1.0.0-1.x86_64.rpm");
        assert_eq!(pick(host(Os::Linux, Arch::X64, Some("unknown")), LinuxFormat::Auto).unwrap(), "Z_1.0.0_amd64.AppImage");
        assert_eq!(pick(host(Os::Linux, Arch::X64, Some("debian")), LinuxFormat::Appimage).unwrap(), "Z_1.0.0_amd64.AppImage");
        assert_eq!(pick(host(Os::Windows, Arch::X64, None), LinuxFormat::Auto).unwrap(), "Z_1.0.0_x64-setup.exe");
        assert_eq!(pick(host(Os::Macos, Arch::Arm64, None), LinuxFormat::Auto).unwrap(), "Z_1.0.0_aarch64.dmg");
        assert!(pick(host(Os::Linux, Arch::Arm64, Some("debian")), LinuxFormat::Auto).is_none());
    }
}
