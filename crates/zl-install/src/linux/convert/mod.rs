pub mod debbuild;
pub mod depmap;
pub mod extract;
pub mod pkgbuild;
pub mod rpmspec;

use std::path::{Path, PathBuf};

pub struct BuildDir {
    path: PathBuf,
    keep: bool,
}

impl BuildDir {
    pub fn create(name: &str) -> std::io::Result<BuildDir> {
        let path =
            crate::linux::registry::build_dir().join(format!("{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&path)?;
        Ok(BuildDir { path, keep: false })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for BuildDir {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
