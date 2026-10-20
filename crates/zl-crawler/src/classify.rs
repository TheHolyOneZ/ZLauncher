use std::sync::OnceLock;

use regex::Regex;

use crate::model::{Arch, Kind, Os};

#[derive(Debug, Clone, PartialEq)]
pub struct Classified {
    pub os: Os,
    pub arch: Option<Arch>,
    pub kind: Kind,
    pub version: Option<String>,
}

fn is_side_file(lower: &str) -> bool {
    const SIDE: &[&str] = &[
        ".sig", ".sha256", ".sha512", ".md5", ".asc", ".txt", ".json", ".sh", ".so", ".html", ".php",
        ".md", ".png", ".jpg", ".svg", ".webp", ".mp4", ".css", ".js", ".xml", ".ico", ".pdf",
    ];
    SIDE.iter().any(|s| lower.ends_with(s)) || lower.starts_with("sha256sums") || lower == "checksums"
}

fn has_token(lower: &str, tokens: &[&str]) -> bool {
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|t| tokens.contains(&t))
}

fn arch_of(lower: &str) -> Option<Arch> {
    if lower.contains("x86_64") || has_token(lower, &["amd64", "x64", "win64", "x86"]) {
        Some(Arch::X64)
    } else if has_token(lower, &["aarch64", "arm64"]) || lower.contains("apple-silicon") {
        Some(Arch::Arm64)
    } else if has_token(lower, &["universal"]) {
        Some(Arch::Universal)
    } else {
        None
    }
}

fn version_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?:^|[^0-9.])v?(\d+\.\d+(?:\.\d+)?(?:-(?:alpha|beta|rc|pre)\.?\d*)?)")
            .unwrap()
    })
}

pub fn version_of(name: &str) -> Option<String> {
    version_re().captures(name).map(|c| c[1].to_string())
}

pub fn classify(filename: &str) -> Option<Classified> {
    let lower = filename.to_ascii_lowercase();
    if lower.is_empty() || is_side_file(&lower) {
        return None;
    }
    let browser = has_token(&lower, &["chromium", "chrome", "firefox", "edge", "extension", "addon", "xpi", "crx"]);
    let win = has_token(&lower, &["windows", "win", "win64", "win32"]);
    let mac = has_token(&lower, &["macos", "darwin", "mac", "osx"]);
    let linux = has_token(&lower, &["linux"]);

    let (kind, os) = if lower.ends_with(".deb") {
        (Kind::Deb, Os::Linux)
    } else if lower.ends_with(".rpm") {
        (Kind::Rpm, Os::Linux)
    } else if lower.ends_with(".appimage") {
        (Kind::AppImage, Os::Linux)
    } else if lower.ends_with(".pkg.tar.zst") || lower.ends_with(".pkg.tar.xz") {
        (Kind::Pacman, Os::Linux)
    } else if lower.ends_with(".msi") {
        (Kind::Msi, Os::Windows)
    } else if lower.ends_with(".exe") {
        let setup = lower.contains("setup") || lower.contains("install");
        (if setup { Kind::Setup } else { Kind::PortableExe }, Os::Windows)
    } else if lower.ends_with(".dmg") {
        (Kind::Dmg, Os::Macos)
    } else if lower.ends_with(".app.tar.gz") || lower.ends_with(".app.zip") {
        (Kind::AppArchive, Os::Macos)
    } else if lower.ends_with(".apk") {
        (Kind::Apk, Os::Android)
    } else if lower.ends_with(".ipa") {
        (Kind::Ipa, Os::Ios)
    } else if lower.ends_with(".xpi") || lower.ends_with(".crx") {
        (Kind::Extension, Os::Browser)
    } else if lower.ends_with(".zip") {
        if browser {
            (Kind::Extension, Os::Browser)
        } else if mac {
            (Kind::Zip, Os::Macos)
        } else if linux {
            (Kind::Zip, Os::Linux)
        } else {
            (Kind::Zip, Os::Windows)
        }
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") || lower.ends_with(".tar.xz") {
        if mac {
            (Kind::TarGz, Os::Macos)
        } else if win {
            (Kind::TarGz, Os::Windows)
        } else {
            (Kind::TarGz, Os::Linux)
        }
    } else if !lower.contains('.') || lower.rsplit('.').next().is_some_and(|e| e.chars().all(|c| c.is_ascii_digit())) {
        if linux {
            (Kind::Binary, Os::Linux)
        } else if mac {
            (Kind::Binary, Os::Macos)
        } else if win {
            (Kind::Binary, Os::Windows)
        } else {
            return None;
        }
    } else {
        return None;
    };

    Some(Classified { os, arch: arch_of(&lower), kind, version: version_of(filename) })
}

pub fn cmp_versions(a: &str, b: &str) -> std::cmp::Ordering {
    fn norm(v: &str) -> Option<semver::Version> {
        let v = v.trim().trim_start_matches('v');
        semver::Version::parse(v).ok().or_else(|| {
            let mut parts: Vec<&str> = v.splitn(2, '-').collect();
            let pre = if parts.len() == 2 { parts.pop() } else { None };
            let nums: Vec<&str> = parts[0].split('.').collect();
            let padded = match nums.len() {
                1 => format!("{}.0.0", nums[0]),
                2 => format!("{}.{}.0", nums[0], nums[1]),
                _ => nums[..3].join("."),
            };
            semver::Version::parse(&match pre {
                Some(p) => format!("{padded}-{p}"),
                None => padded,
            })
            .ok()
        })
    }
    match (norm(a), norm(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => a.cmp(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str) -> (Os, Option<Arch>, Kind, Option<String>) {
        let r = classify(name).unwrap_or_else(|| panic!("{name} not classified"));
        (r.os, r.arch, r.kind, r.version)
    }

    #[test]
    fn tauri_names() {
        assert_eq!(c("ZWebWrap_0.1.0_amd64.deb"), (Os::Linux, Some(Arch::X64), Kind::Deb, Some("0.1.0".into())));
        assert_eq!(c("ZWebWrap-0.1.0-1.x86_64.rpm"), (Os::Linux, Some(Arch::X64), Kind::Rpm, Some("0.1.0".into())));
        assert_eq!(c("ZWebWrap_0.1.0_amd64.AppImage").2, Kind::AppImage);
        assert_eq!(c("ZWebWrap_0.1.0_x64_en-US.msi"), (Os::Windows, Some(Arch::X64), Kind::Msi, Some("0.1.0".into())));
        assert_eq!(c("ZWebWrap_0.1.0_x64-setup.exe").2, Kind::Setup);
        assert_eq!(c("ZyntaxAI_1.0.2_aarch64.dmg"), (Os::Macos, Some(Arch::Arm64), Kind::Dmg, Some("1.0.2".into())));
        assert_eq!(c("ZyntaxAI_1.0.2_x86_64.app.tar.gz").2, Kind::AppArchive);
        assert_eq!(c("ZBase Studio_0.2.0_amd64.deb").3, Some("0.2.0".into()));
    }

    #[test]
    fn other_names() {
        assert_eq!(c("zhell-0.1.1-windows-x86_64.zip").0, Os::Windows);
        assert_eq!(c("zhell-0.1.1-linux-x86_64.tar.gz"), (Os::Linux, Some(Arch::X64), Kind::TarGz, Some("0.1.1".into())));
        assert_eq!(c("zpassword-chromium-1.3.1.zip").2, Kind::Extension);
        assert_eq!(c("zdd-linux-x64"), (Os::Linux, Some(Arch::X64), Kind::Binary, None));
        assert_eq!(c("ZCashflow_1.0.0.apk").0, Os::Android);
        assert_eq!(c("ZDBF-Launcher-1.0.0-Setup.exe").2, Kind::Setup);
        assert_eq!(c("Zyringe.AppImage"), (Os::Linux, None, Kind::AppImage, None));
        assert_eq!(c("ZCodeSnippets_1.0.0_arm64-setup.exe").1, Some(Arch::Arm64));
        assert_eq!(c("ZygnalBotLauncherSetup1.2.0.exe"), (Os::Windows, None, Kind::Setup, Some("1.2.0".into())));
        assert_eq!(c("ZCashflow_1.0.0_macos_apple-silicon.dmg").1, Some(Arch::Arm64));
        assert_eq!(c("zircon-0.10.0-win64.zip").3, Some("0.10.0".into()));
    }

    #[test]
    fn side_files_are_ignored() {
        for n in ["SHA256SUMS", "SHA256SUMS.txt", "x.AppImage.sig", "install.sh", "libzyringe.so", "zyringe-inject", "latest.json"] {
            assert!(classify(n).is_none(), "{n}");
        }
    }

    #[test]
    fn version_order() {
        use std::cmp::Ordering::*;
        assert_eq!(cmp_versions("1.10.0", "1.9.3"), Greater);
        assert_eq!(cmp_versions("1.2", "1.2.0"), Equal);
        assert_eq!(cmp_versions("1.0.0-beta", "1.0.0"), Less);
        assert_eq!(cmp_versions("v2.0", "1.9"), Greater);
    }
}
