use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::linux::distro::{find_library, Family};
use crate::linux::model::{DepResolution, DepStatus, PackageKind, RawDependency};

const TABLE: &str = include_str!("../../../depmap.toml");

#[derive(Deserialize, Debug, Default, Clone)]
struct Entry {
    #[serde(default)]
    arch: Vec<String>,
    #[serde(default)]
    debian: Vec<String>,
    #[serde(default)]
    fedora: Vec<String>,
    #[serde(default)]
    sonames: Vec<String>,
}

#[derive(Deserialize, Debug, Default)]
struct Table {
    #[serde(default)]
    base: Vec<String>,
    #[serde(default, rename = "map")]
    entries: Vec<Entry>,
}

pub struct DepMap {
    entries: Vec<Entry>,
    base: HashSet<String>,
}

impl DepMap {
    pub fn builtin() -> &'static DepMap {
        static MAP: OnceLock<DepMap> = OnceLock::new();
        MAP.get_or_init(|| {
            let table: Table = toml::from_str(TABLE).expect("depmap.toml is valid");
            DepMap {
                entries: table.entries,
                base: table.base.into_iter().collect(),
            }
        })
    }

    fn entry_for(&self, name: &str, source: Family) -> Option<&Entry> {
        self.entries.iter().find(|e| {
            let names: &[String] = match source {
                Family::Debian => &e.debian,
                Family::Fedora | Family::Suse => &e.fedora,
                Family::Arch => &e.arch,
                Family::Unknown => &[],
            };
            names.iter().any(|n| n == name) || e.sonames.iter().any(|s| s == name)
        })
    }

    fn target_name(entry: &Entry, target: Family) -> Option<String> {
        match target {
            Family::Arch => entry.arch.first().cloned(),
            Family::Debian => entry.debian.first().cloned(),
            Family::Fedora | Family::Suse => entry
                .sonames
                .first()
                .map(|s| soname_capability(s))
                .or_else(|| entry.fedora.first().cloned()),
            Family::Unknown => None,
        }
    }

    pub fn resolve(
        &self,
        dep: &RawDependency,
        source: PackageKind,
        target: Family,
        verifier: &dyn Verifier,
    ) -> DepResolution {
        let source_family = match source {
            PackageKind::Deb => Family::Debian,
            PackageKind::Rpm => Family::Fedora,
            PackageKind::AppImage => Family::Unknown,
        };
        let raw = dep.name.clone();
        let candidates: Vec<String> = if dep.alternatives.is_empty() {
            vec![raw.clone()]
        } else {
            dep.alternatives.clone()
        };

        for alt in &candidates {
            let name = canonical_name(alt);

            if self.base.contains(&name)
                || self.base.contains(alt.as_str())
                || is_core_soname(&name)
            {
                return DepResolution {
                    raw,
                    resolved: None,
                    status: DepStatus::Skipped {
                        reason: "part of every system".into(),
                    },
                };
            }
            if alt.starts_with('/') {
                return DepResolution {
                    raw,
                    resolved: None,
                    status: DepStatus::Skipped {
                        reason: "file requirement".into(),
                    },
                };
            }
            if let Some(entry) = self.entry_for(&name, source_family) {
                if let Some(t) = Self::target_name(entry, target) {
                    return DepResolution {
                        raw,
                        resolved: Some(t),
                        status: DepStatus::Found,
                    };
                }

                continue;
            }
        }

        for alt in &candidates {
            let name = canonical_name(alt);
            for candidate in heuristic_candidates(&name, source_family, target) {
                if verifier.exists(&candidate) {
                    return DepResolution {
                        raw,
                        resolved: Some(candidate),
                        status: DepStatus::Found,
                    };
                }
            }
            if let Some(found) = verifier.search(&name) {
                return DepResolution {
                    raw,
                    resolved: Some(found),
                    status: DepStatus::Found,
                };
            }
            if name.contains(".so") && find_library(&name).is_some() {
                return DepResolution {
                    raw,
                    resolved: None,
                    status: DepStatus::Skipped {
                        reason: "already on this system".into(),
                    },
                };
            }
        }

        DepResolution {
            raw,
            resolved: None,
            status: DepStatus::Unresolved,
        }
    }

    pub fn resolve_all(
        &self,
        deps: &[RawDependency],
        source: PackageKind,
        target: Family,
        verifier: &dyn Verifier,
    ) -> Vec<DepResolution> {
        let mut seen = HashSet::new();
        deps.iter()
            .map(|d| self.resolve(d, source, target, verifier))
            .filter(|r| seen.insert((r.raw.clone(), r.resolved.clone())))
            .collect()
    }
}

pub fn soname_capability(soname: &str) -> String {
    if std::mem::size_of::<usize>() == 8 {
        format!("{soname}()(64bit)")
    } else {
        soname.to_string()
    }
}

pub fn canonical_name(raw: &str) -> String {
    let base = raw.split('(').next().unwrap_or(raw).trim();
    base.split_whitespace().next().unwrap_or(base).to_string()
}

fn is_core_soname(name: &str) -> bool {
    name.starts_with("libc.so.")
        || name.starts_with("libm.so.")
        || name.starts_with("libpthread.so.")
        || name.starts_with("libdl.so.")
        || name.starts_with("librt.so.")
        || name.starts_with("ld-linux")
}

pub fn heuristic_candidates(name: &str, source: Family, target: Family) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    let lower = name.to_ascii_lowercase();
    push(name.to_string());
    if lower != name {
        push(lower.clone());
    }

    if source == Family::Debian {
        let mut s = lower.trim_end_matches("t64").to_string();
        push(s.clone());
        loop {
            let trimmed = strip_version_suffix(&s);
            if trimmed == s {
                break;
            }
            s = trimmed;
            push(s.clone());
        }
        if matches!(target, Family::Fedora | Family::Suse | Family::Arch) {
            if let Some(stripped) = s.strip_prefix("lib") {
                push(stripped.to_string());
            }
        }
    }
    if matches!(source, Family::Fedora | Family::Suse) && target == Family::Arch {
        push(lower.clone());
    }
    if target == Family::Debian && !lower.starts_with("lib") {
        push(format!("lib{lower}"));
    }
    out
}

fn strip_version_suffix(s: &str) -> String {
    let t = s.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    let t = t.trim_end_matches('-');
    if t.len() < s.len() && !t.is_empty() {
        t.to_string()
    } else {
        s.to_string()
    }
}

pub trait Verifier {
    fn exists(&self, name: &str) -> bool;

    fn search(&self, _name: &str) -> Option<String> {
        None
    }
}

pub struct NoVerifier;
impl Verifier for NoVerifier {
    fn exists(&self, _name: &str) -> bool {
        false
    }
}

pub struct HostVerifier {
    family: Family,
    cache: RefCell<HashMap<String, bool>>,
}

impl HostVerifier {
    pub fn new(family: Family) -> Self {
        HostVerifier {
            family,
            cache: RefCell::new(HashMap::new()),
        }
    }

    fn query(&self, name: &str) -> bool {
        let ok = match self.family {
            Family::Arch => Command::new("pacman")
                .args(["-Si", "--", name])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false),
            Family::Debian => Command::new("apt-cache")
                .args(["show", "--no-all-versions", "--", name])
                .output()
                .map(|o| o.status.success() && !o.stdout.is_empty())
                .unwrap_or(false),
            Family::Fedora => {
                Command::new("dnf")
                    .args(["-C", "-q", "info", "--", name])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                    || Command::new("rpm")
                        .args(["-q", "--", name])
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false)
            }
            Family::Suse => {
                Command::new("zypper")
                    .args(["--non-interactive", "-q", "info", "--", name])
                    .output()
                    .map(|o| {
                        o.status.success() && String::from_utf8_lossy(&o.stdout).contains("Version")
                    })
                    .unwrap_or(false)
                    || Command::new("rpm")
                        .args(["-q", "--", name])
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false)
            }
            Family::Unknown => false,
        };
        ok
    }
}

impl Verifier for HostVerifier {
    fn exists(&self, name: &str) -> bool {
        if name.is_empty() || name.contains(char::is_whitespace) {
            return false;
        }
        if let Some(v) = self.cache.borrow().get(name) {
            return *v;
        }
        let v = self.query(name);
        self.cache.borrow_mut().insert(name.to_string(), v);
        v
    }

    fn search(&self, name: &str) -> Option<String> {
        if self.family != Family::Debian {
            return None;
        }
        let pattern = format!("^{}[0-9.t-]*$", regex_escape(name));
        let out = Command::new("apt-cache")
            .args(["search", "--names-only", &pattern])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines()
            .filter_map(|l| l.split_whitespace().next())
            .filter(|n| !n.ends_with("-dev") && !n.ends_with("-dbg"))
            .min_by_key(|n| n.len())
            .map(str::to_string)
    }
}

fn regex_escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if ".+*?()[]{}|^$\\".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(name: &str) -> RawDependency {
        RawDependency {
            name: name.into(),
            version_req: None,
            alternatives: vec![],
        }
    }

    #[test]
    fn table_hits() {
        let m = DepMap::builtin();
        let r = m.resolve(
            &dep("libgtk-3-0"),
            PackageKind::Deb,
            Family::Arch,
            &NoVerifier,
        );
        assert_eq!(r.resolved.as_deref(), Some("gtk3"));
        let r = m.resolve(
            &dep("libgtk-3.so.0()(64bit)"),
            PackageKind::Rpm,
            Family::Arch,
            &NoVerifier,
        );
        assert_eq!(r.resolved.as_deref(), Some("gtk3"));
        let r = m.resolve(&dep("gtk3"), PackageKind::Rpm, Family::Debian, &NoVerifier);
        assert_eq!(r.resolved.as_deref(), Some("libgtk-3-0t64"));
        let r = m.resolve(
            &dep("libwebkit2gtk-4.1-0"),
            PackageKind::Deb,
            Family::Fedora,
            &NoVerifier,
        );
        assert_eq!(
            r.resolved.as_deref(),
            Some(soname_capability("libwebkit2gtk-4.1.so.0").as_str())
        );
        let r = m.resolve(
            &dep("xdg-utils"),
            PackageKind::Deb,
            Family::Suse,
            &NoVerifier,
        );
        assert_eq!(r.resolved.as_deref(), Some("xdg-utils"));
    }

    #[test]
    fn base_and_files_are_skipped() {
        let m = DepMap::builtin();
        assert!(matches!(
            m.resolve(&dep("libc6"), PackageKind::Deb, Family::Arch, &NoVerifier)
                .status,
            DepStatus::Skipped { .. }
        ));
        assert!(matches!(
            m.resolve(
                &dep("libc.so.6(GLIBC_2.34)(64bit)"),
                PackageKind::Rpm,
                Family::Arch,
                &NoVerifier
            )
            .status,
            DepStatus::Skipped { .. }
        ));
        assert!(matches!(
            m.resolve(&dep("/bin/sh"), PackageKind::Rpm, Family::Arch, &NoVerifier)
                .status,
            DepStatus::Skipped { .. }
        ));
    }

    #[test]
    fn alternatives_fall_through_to_a_resolvable_one() {
        let m = DepMap::builtin();
        let d = RawDependency {
            name: "libgconf-2-4".into(),
            version_req: None,
            alternatives: vec!["libgconf-2-4".into(), "libnotify4".into()],
        };
        let r = m.resolve(&d, PackageKind::Deb, Family::Arch, &NoVerifier);
        assert_eq!(r.resolved.as_deref(), Some("libnotify"));
    }

    #[test]
    fn heuristics_strip_debian_suffixes() {
        let c = heuristic_candidates("libsecret-1-0", Family::Debian, Family::Arch);
        assert!(c.contains(&"libsecret".to_string()));
        let c = heuristic_candidates("libglib2.0-0t64", Family::Debian, Family::Fedora);
        assert!(c.contains(&"libglib".to_string()) && c.contains(&"glib".to_string()));
    }

    #[test]
    fn unresolved_when_nothing_matches() {
        let m = DepMap::builtin();
        let r = m.resolve(
            &dep("libtotallymadeup9"),
            PackageKind::Deb,
            Family::Arch,
            &NoVerifier,
        );
        assert!(matches!(r.status, DepStatus::Unresolved));
    }
}
