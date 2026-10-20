use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::linux::inspect;
use crate::linux::model::{InstalledEntry, PackageKind, PackageModel};
use crate::linux::registry::{data_dir, Registry};
use crate::linux::settings::Settings;
use crate::linux::version::{relation, Relation};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Exact,

    Name,

    Fuzzy,

    Folder,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMatch {
    pub entry_id: String,
    pub entry_name: String,
    pub installed_version: String,
    pub relation: Relation,
    pub confidence: Confidence,
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

pub fn name_from_filename(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = String::new();
    for (i, seg) in stem.split(['-', '_', ' ']).enumerate() {
        let looks_versiony = seg
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
            || (seg.starts_with(['v', 'V']) && seg[1..].starts_with(|c: char| c.is_ascii_digit()))
            || matches!(
                seg.to_ascii_lowercase().as_str(),
                "amd64"
                    | "x86"
                    | "x86_64"
                    | "x64"
                    | "arm64"
                    | "aarch64"
                    | "linux"
                    | "all"
                    | "noarch"
            );
        if looks_versiony && i > 0 {
            break;
        }

        let cut = seg
            .char_indices()
            .find(|&(j, c)| c.is_ascii_digit() && j > 0)
            .map(|(j, _)| j)
            .unwrap_or(seg.len());
        let mut head = &seg[..cut];
        if head.len() > 1 && head.ends_with(['v', 'V']) && cut < seg.len() {
            head = &head[..head.len() - 1];
        }
        out.push_str(head);
        if cut < seg.len() {
            break;
        }
    }
    norm(&out)
}

pub fn find_match(model: &PackageModel, registry: &Registry) -> Option<UpdateMatch> {
    let mut best: Option<(Confidence, &InstalledEntry)> = None;
    let file_name = name_from_filename(&model.source_path);
    for e in &registry.entries {
        let c = if norm(&e.slug) == norm(&model.name) {
            Confidence::Exact
        } else if norm(&e.name) == norm(&model.display_name) {
            Confidence::Name
        } else if !file_name.is_empty()
            && file_name.len() >= 4
            && (norm(&e.name) == file_name || norm(&e.slug) == file_name)
        {
            Confidence::Fuzzy
        } else {
            continue;
        };
        if best.map(|(bc, _)| c < bc).unwrap_or(true) {
            best = Some((c, e));
        }
    }
    best.map(|(confidence, e)| UpdateMatch {
        entry_id: e.id.clone(),
        entry_name: e.name.clone(),
        installed_version: e.version.clone(),
        relation: relation(&model.version, &e.version),
        confidence,
    })
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCandidate {
    pub path: PathBuf,
    pub kind: PackageKind,
    pub name: String,
    pub version: String,
    pub folder: PathBuf,

    pub per_app: Option<String>,
    #[serde(flatten)]
    pub matched: UpdateMatch,
}

#[derive(Serialize, Deserialize, Default)]
struct ScanCache {
    files: HashMap<String, CachedFile>,
}

#[derive(Serialize, Deserialize, Clone)]
struct CachedFile {
    size: u64,
    mtime: i64,
    name: String,
    display_name: String,
    version: String,
    kind: PackageKind,
}

fn cache_path() -> PathBuf {
    data_dir().join("update-scan.json")
}

fn is_package_file(p: &Path) -> bool {
    p.extension()
        .map(|e| {
            let e = e.to_string_lossy().to_ascii_lowercase();
            e == "deb" || e == "rpm" || e == "appimage"
        })
        .unwrap_or(false)
}

fn peek(cache: &mut ScanCache, path: &Path) -> Option<CachedFile> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let key = path.to_string_lossy().into_owned();
    if let Some(c) = cache.files.get(&key) {
        if c.size == meta.len() && c.mtime == mtime {
            return Some(c.clone());
        }
    }
    let model = inspect::inspect(path).ok()?;
    let c = CachedFile {
        size: meta.len(),
        mtime,
        name: model.name,
        display_name: model.display_name,
        version: model.version,
        kind: model.kind,
    };
    cache.files.insert(key, c.clone());
    Some(c)
}

fn list_packages(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && is_package_file(p))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

pub fn scan(registry: &Registry, settings: &Settings) -> Vec<UpdateCandidate> {
    let mut cache: ScanCache = std::fs::read_to_string(cache_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let mut out: Vec<UpdateCandidate> = Vec::new();

    if let Some(dir) = settings.update_dir.as_ref().filter(|d| d.is_dir()) {
        for path in list_packages(dir) {
            let Some(c) = peek(&mut cache, &path) else {
                continue;
            };
            let pseudo = PackageModel {
                kind: c.kind,
                source_path: path.clone(),
                file_size: c.size,
                name: c.name.clone(),
                display_name: c.display_name.clone(),
                version: c.version.clone(),
                arch: String::new(),
                summary: None,
                description: None,
                homepage: None,
                license: None,
                maintainer: None,
                installed_size: None,
                dependencies: vec![],
                desktop_entries: vec![],
                icon_data_url: None,
                file_count: 0,
                top_level_paths: vec![],
            };
            if let Some(m) = find_match(&pseudo, registry) {
                let ignored = registry
                    .find(&m.entry_id)
                    .map(|e| e.ignored_versions.contains(&c.version))
                    .unwrap_or(false);
                if m.relation == Relation::Newer && !ignored {
                    out.push(UpdateCandidate {
                        path,
                        kind: c.kind,
                        name: c.display_name,
                        version: c.version,
                        folder: dir.clone(),
                        per_app: None,
                        matched: m,
                    });
                }
            }
        }
    }

    for entry in &registry.entries {
        let Some(dir) = entry.update_dir.as_ref().filter(|d| d.is_dir()) else {
            continue;
        };
        for path in list_packages(dir) {
            let Some(c) = peek(&mut cache, &path) else {
                continue;
            };
            let rel = relation(&c.version, &entry.version);
            if matches!(rel, Relation::Newer | Relation::Unknown)
                && !entry.ignored_versions.contains(&c.version)
            {
                out.push(UpdateCandidate {
                    path,
                    kind: c.kind,
                    name: c.display_name,
                    version: c.version,
                    folder: dir.clone(),
                    per_app: Some(entry.id.clone()),
                    matched: UpdateMatch {
                        entry_id: entry.id.clone(),
                        entry_name: entry.name.clone(),
                        installed_version: entry.version.clone(),
                        relation: rel,
                        confidence: Confidence::Folder,
                    },
                });
            }
        }
    }

    out.sort_by(|a, b| {
        a.matched
            .entry_id
            .cmp(&b.matched.entry_id)
            .then_with(|| crate::linux::version::compare(&b.version, &a.version))
    });
    out.dedup_by(|a, b| a.matched.entry_id == b.matched.entry_id);

    if let Some(parent) = cache_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(cache_path(), serde_json::to_vec(&cache).unwrap_or_default());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_names() {
        assert_eq!(
            name_from_filename(Path::new("/x/imagetoolv1.2.0.AppImage")),
            "imagetool"
        );
        assert_eq!(
            name_from_filename(Path::new("/x/ZFontManager_0.6.0_amd64.deb")),
            "zfontmanager"
        );
        assert_eq!(
            name_from_filename(Path::new("/x/hello-2.10-9.fc38.x86_64.rpm")),
            "hello"
        );
        assert_eq!(
            name_from_filename(Path::new("/x/Obsidian-1.6.7.AppImage")),
            "obsidian"
        );
        assert_eq!(name_from_filename(Path::new("/x/discord.deb")), "discord");
    }
}
