use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    Arch,
    Debian,
    Fedora,
    Suse,
    Unknown,
}

impl Family {
    pub fn label(self) -> &'static str {
        match self {
            Family::Arch => "Arch",
            Family::Debian => "Debian",
            Family::Fedora => "Fedora",
            Family::Suse => "openSUSE",
            Family::Unknown => "Unknown",
        }
    }

    pub fn uses_rpm(self) -> bool {
        matches!(self, Family::Fedora | Family::Suse)
    }

    pub fn from_os_release(id: &str, id_like: &str) -> Family {
        let ids: Vec<&str> = std::iter::once(id)
            .chain(id_like.split_whitespace())
            .collect();
        for candidate in ids {
            match candidate {
                "arch" | "archlinux" | "endeavouros" | "manjaro" | "cachyos" | "garuda"
                | "artix" => return Family::Arch,
                "debian" | "ubuntu" | "linuxmint" | "pop" | "elementary" | "zorin" | "kali"
                | "raspbian" | "neon" => return Family::Debian,
                "fedora" | "rhel" | "centos" | "rocky" | "almalinux" | "nobara" | "bazzite"
                | "ultramarine" => return Family::Fedora,
                "opensuse"
                | "opensuse-leap"
                | "opensuse-tumbleweed"
                | "opensuse-slowroll"
                | "sles"
                | "suse" => return Family::Suse,
                _ => {}
            }
        }
        Family::Unknown
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub name: String,
    pub kind: String,
    pub path: Option<PathBuf>,
    pub required_for: String,

    pub package: Option<String>,
    pub optional: bool,
}

impl ToolStatus {
    pub fn present(&self) -> bool {
        self.path.is_some()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub family: Family,
    pub id: String,
    pub pretty_name: String,
    pub id_like: String,
    pub desktop: Option<String>,
    pub session: Option<String>,
    pub arch: String,
    pub is_root: bool,
    pub tools: Vec<ToolStatus>,
}

impl SystemInfo {
    pub fn tool(&self, name: &str) -> Option<&ToolStatus> {
        self.tools.iter().find(|t| t.name == name)
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.tool(name).map(ToolStatus::present).unwrap_or(false)
    }

    pub fn missing_required(&self, required_for: &[&str]) -> Vec<ToolStatus> {
        self.tools
            .iter()
            .filter(|t| {
                !t.optional && !t.present() && required_for.contains(&t.required_for.as_str())
            })
            .cloned()
            .collect()
    }
}

pub fn parse_os_release(content: &str) -> HashMap<String, String> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (k, v) = line.split_once('=')?;
            let v = v.trim().trim_matches('"').trim_matches('\'');
            Some((k.trim().to_string(), v.to_string()))
        })
        .collect()
}

pub fn which(bin: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for extra in [
        "/usr/local/sbin",
        "/usr/local/bin",
        "/usr/sbin",
        "/usr/bin",
        "/sbin",
        "/bin",
    ] {
        dirs.push(PathBuf::from(extra));
    }
    dirs.into_iter()
        .map(|d| d.join(bin))
        .find(|p| is_executable(p))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

pub fn find_library(soname: &str) -> Option<PathBuf> {
    const DIRS: &[&str] = &[
        "/usr/lib",
        "/usr/lib64",
        "/lib",
        "/lib64",
        "/usr/lib/x86_64-linux-gnu",
        "/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/lib/aarch64-linux-gnu",
        "/usr/local/lib",
    ];
    DIRS.iter()
        .map(|d| Path::new(d).join(soname))
        .find(|p| p.exists())
}

fn tool(name: &str, required_for: &str, package: Option<&str>, optional: bool) -> ToolStatus {
    ToolStatus {
        name: name.to_string(),
        kind: "binary".into(),
        path: which(name),
        required_for: required_for.to_string(),
        package: package.map(str::to_string),
        optional,
    }
}

fn library(soname: &str, required_for: &str, package: Option<&str>) -> ToolStatus {
    ToolStatus {
        name: soname.to_string(),
        kind: "library".into(),
        path: find_library(soname),
        required_for: required_for.to_string(),
        package: package.map(str::to_string),
        optional: true,
    }
}

fn tools_for(family: Family) -> Vec<ToolStatus> {
    let mut tools = vec![tool(
        "pkexec",
        "install",
        Some(match family {
            Family::Arch => "polkit",
            Family::Debian => "policykit-1",
            Family::Fedora => "polkit",
            Family::Suse => "polkit",
            Family::Unknown => "polkit",
        }),
        false,
    )];

    match family {
        Family::Arch => {
            tools.push(tool("pacman", "install", None, false));
            tools.push(tool("makepkg", "convert", Some("base-devel"), false));
            tools.push(tool("fakeroot", "convert", Some("base-devel"), false));
        }
        Family::Debian => {
            tools.push(tool("apt-get", "install", None, false));
            tools.push(tool("dpkg-deb", "convert", Some("dpkg"), false));
        }
        Family::Fedora => {
            tools.push(tool("dnf", "install", None, false));
            tools.push(tool("rpmbuild", "convert", Some("rpm-build"), false));
        }
        Family::Suse => {
            tools.push(tool("zypper", "install", None, false));
            tools.push(tool("rpmbuild", "convert", Some("rpm-build"), false));
        }
        Family::Unknown => {}
    }

    let fuse_pkg = match family {
        Family::Arch => "fuse2",
        Family::Debian => "libfuse2t64",
        Family::Fedora => "fuse-libs",
        Family::Suse => "libfuse2",
        Family::Unknown => "fuse2",
    };
    tools.push(library("libfuse.so.2", "appimage", Some(fuse_pkg)));
    tools.push(tool("update-desktop-database", "shortcuts", None, true));
    tools.push(tool("xdg-user-dir", "shortcuts", None, true));
    tools.push(tool("gio", "shortcuts", None, true));
    tools
}

pub fn detect_system() -> SystemInfo {
    let os_release = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    let map = parse_os_release(&os_release);
    let id = map.get("ID").cloned().unwrap_or_default();
    let id_like = map.get("ID_LIKE").cloned().unwrap_or_default();
    let family = Family::from_os_release(&id, &id_like);
    let is_root = unsafe { libc::geteuid() } == 0;

    SystemInfo {
        family,
        pretty_name: map
            .get("PRETTY_NAME")
            .or_else(|| map.get("NAME"))
            .cloned()
            .unwrap_or_else(|| "Linux".into()),
        id,
        id_like,
        desktop: std::env::var("XDG_CURRENT_DESKTOP")
            .ok()
            .filter(|s| !s.is_empty()),
        session: std::env::var("XDG_SESSION_TYPE")
            .ok()
            .filter(|s| !s.is_empty()),
        arch: std::env::consts::ARCH.to_string(),
        is_root,
        tools: tools_for(family),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_from_id_like() {
        assert_eq!(Family::from_os_release("endeavouros", "arch"), Family::Arch);
        assert_eq!(Family::from_os_release("ubuntu", "debian"), Family::Debian);
        assert_eq!(
            Family::from_os_release("linuxmint", "ubuntu debian"),
            Family::Debian
        );
        assert_eq!(Family::from_os_release("nobara", "fedora"), Family::Fedora);
        assert_eq!(
            Family::from_os_release("rocky", "rhel centos fedora"),
            Family::Fedora
        );
        assert_eq!(Family::from_os_release("nixos", ""), Family::Unknown);
    }

    #[test]
    fn os_release_parsing_strips_quotes() {
        let m = parse_os_release("NAME=\"EndeavourOS\"\nID=endeavouros\n# c\nID_LIKE='arch'\n");
        assert_eq!(m["NAME"], "EndeavourOS");
        assert_eq!(m["ID_LIKE"], "arch");
    }
}
