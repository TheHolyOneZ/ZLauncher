use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::linux::model::InstalledEntry;

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct Registry {
    pub version: u32,
    pub entries: Vec<InstalledEntry>,
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("zlauncher")
}

pub fn registry_path() -> PathBuf {
    data_dir().join("registry.json")
}

pub fn icons_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("icons")
        .join("zlauncher")
}

pub fn applications_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("applications")
}

pub fn build_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("zlauncher")
        .join("build")
}

impl Registry {
    pub fn load() -> Registry {
        std::fs::read_to_string(registry_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| Registry {
                version: 1,
                entries: Vec::new(),
            })
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = registry_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }

    pub fn find(&self, id: &str) -> Option<&InstalledEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn find_by_slug(&self, slug: &str) -> Option<&InstalledEntry> {
        self.entries.iter().find(|e| e.slug == slug)
    }

    pub fn upsert(&mut self, entry: InstalledEntry) {
        self.entries.retain(|e| e.id != entry.id);
        self.entries.insert(0, entry);
    }

    pub fn remove(&mut self, id: &str) -> Option<InstalledEntry> {
        let idx = self.entries.iter().position(|e| e.id == id)?;
        Some(self.entries.remove(idx))
    }
}

pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}
