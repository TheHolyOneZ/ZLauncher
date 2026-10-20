use std::path::Path;

use crate::linux::inspect::deb::walk_members;
use crate::linux::inspect::{InspectError, Result};
use crate::linux::model::PackageKind;

pub fn extract_tree(kind: PackageKind, source: &Path, dest: &Path) -> Result<()> {
    match kind {
        PackageKind::Deb => extract_deb(source, dest),
        PackageKind::Rpm => crate::linux::inspect::rpm::open(source)?
            .extract(dest)
            .map_err(|e| InspectError::Rpm(format!("extracting payload: {e}"))),
        PackageKind::AppImage => Err(InspectError::AppImage(
            "AppImages are integrated, not extracted".into(),
        )),
    }
}

fn extract_deb(source: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest).map_err(|e| InspectError::Io(dest.display().to_string(), e))?;
    let mut extracted = false;
    walk_members(source, |name, reader| {
        if name.starts_with("data.tar") {
            let mut archive = tar::Archive::new(reader);
            archive.set_preserve_permissions(true);
            archive.set_preserve_mtime(true);
            archive.set_unpack_xattrs(false);
            archive.set_overwrite(true);
            archive
                .unpack(dest)
                .map_err(|e| InspectError::Deb(format!("unpacking data.tar: {e}")))?;
            extracted = true;
        }
        Ok(())
    })?;
    if !extracted {
        return Err(InspectError::Deb("no data.tar member".into()));
    }
    Ok(())
}

pub fn list_tree(root: &Path) -> std::io::Result<Vec<(String, bool, bool)>> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, bool, bool)>) -> std::io::Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path)?;
            let rel = format!(
                "/{}",
                path.strip_prefix(base).unwrap_or(&path).to_string_lossy()
            );
            let is_link = meta.file_type().is_symlink();
            let is_dir = meta.is_dir() && !is_link;
            out.push((rel, is_dir, is_link));
            if is_dir {
                walk(base, &path, out)?;
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

pub fn tree_size(root: &Path) -> u64 {
    fn walk(dir: &Path) -> u64 {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| {
                        let p = e.path();
                        match std::fs::symlink_metadata(&p) {
                            Ok(m) if m.is_dir() => walk(&p),
                            Ok(m) if m.is_file() => m.len(),
                            _ => 0,
                        }
                    })
                    .sum()
            })
            .unwrap_or(0)
    }
    walk(root)
}
