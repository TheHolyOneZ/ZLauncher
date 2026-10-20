pub mod appimage;
pub mod deb;
pub mod rpm;

use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

use thiserror::Error;

use crate::linux::model::{DesktopEntry, PackageKind, PackageModel};
use crate::linux::util::{data_url, icon_size_rank, image_mime};

#[derive(Debug, Error)]
pub enum InspectError {
    #[error("could not read {0}: {1}")]
    Io(String, #[source] std::io::Error),
    #[error("{0} is not a .deb, .rpm or AppImage")]
    UnknownFormat(String),
    #[error("malformed .deb: {0}")]
    Deb(String),
    #[error("malformed .rpm: {0}")]
    Rpm(String),
    #[error("malformed AppImage: {0}")]
    AppImage(String),
}

pub type Result<T> = std::result::Result<T, InspectError>;

pub fn detect_kind(path: &Path) -> Result<PackageKind> {
    let mut f =
        std::fs::File::open(path).map_err(|e| InspectError::Io(path.display().to_string(), e))?;
    let mut head = [0u8; 16];
    let n = f
        .read(&mut head)
        .map_err(|e| InspectError::Io(path.display().to_string(), e))?;
    let head = &head[..n];
    if head.starts_with(b"!<arch>\n") {
        return Ok(PackageKind::Deb);
    }
    if head.starts_with(&[0xED, 0xAB, 0xEE, 0xDB]) {
        return Ok(PackageKind::Rpm);
    }
    if head.starts_with(b"\x7fELF") {
        let magic = head.get(8..11) == Some(b"AI\x02") || head.get(8..11) == Some(b"AI\x01");
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().eq_ignore_ascii_case("appimage"))
            .unwrap_or(false);
        if magic || ext {
            return Ok(PackageKind::AppImage);
        }
    }
    Err(InspectError::UnknownFormat(path.display().to_string()))
}

pub fn inspect(path: &Path) -> Result<PackageModel> {
    match detect_kind(path)? {
        PackageKind::Deb => deb::inspect(path),
        PackageKind::Rpm => rpm::inspect(path),
        PackageKind::AppImage => appimage::inspect(path),
    }
}

const MAX_ICON_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Default)]
pub struct PayloadScan {
    pub file_count: usize,
    pub top_level: BTreeSet<String>,
    pub desktop_entries: Vec<DesktopEntry>,

    pub icons: Vec<(String, Vec<u8>)>,
}

impl PayloadScan {
    pub fn wants_content(path: &str, size: u64) -> bool {
        is_desktop_path(path) || (is_icon_path(path) && size <= MAX_ICON_BYTES)
    }

    pub fn visit(&mut self, path: &str, is_dir: bool, content: Option<&[u8]>) {
        let path = path.trim_start_matches("./").trim_start_matches('/');
        if path.is_empty() {
            return;
        }
        if !is_dir {
            self.file_count += 1;
        }
        let mut parts = path.split('/');
        if let (Some(a), Some(b)) = (parts.next(), parts.next()) {
            if parts.next().is_some() || !is_dir {
                self.top_level.insert(format!("{a}/{b}"));
            }
        }
        let Some(content) = content else { return };
        if is_desktop_path(path) {
            if let Some(entry) = crate::linux::desktop::parse(path, &String::from_utf8_lossy(content)) {
                self.desktop_entries.push(entry);
            }
        } else if is_icon_path(path) {
            self.icons.push((path.to_string(), content.to_vec()));
        }
    }

    pub fn primary_desktop(&self, package_name: &str) -> Option<&DesktopEntry> {
        let visible: Vec<&DesktopEntry> = self
            .desktop_entries
            .iter()
            .filter(|d| !d.no_display)
            .collect();
        let pool: &[&DesktopEntry] = if visible.is_empty() { &[] } else { &visible };
        pool.iter()
            .find(|d| {
                let stem = Path::new(&d.path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_lowercase());
                stem.as_deref() == Some(&package_name.to_lowercase())
            })
            .or_else(|| pool.first())
            .copied()
            .or_else(|| self.desktop_entries.first())
    }

    pub fn icon_data_url(
        &self,
        entry: Option<&DesktopEntry>,
        package_name: &str,
    ) -> Option<String> {
        let wanted = entry.and_then(|e| e.icon.clone());
        let matches = |p: &str| -> bool {
            match &wanted {
                Some(w) if w.starts_with('/') => p == w.trim_start_matches('/'),
                Some(w) => stem_eq(p, w),
                None => stem_eq(p, package_name),
            }
        };
        let best = self
            .icons
            .iter()
            .filter(|(p, _)| image_mime(p).is_some())
            .filter(|(p, _)| matches(p))
            .max_by_key(|(p, _)| icon_size_rank(p))
            .or_else(|| {
                self.icons
                    .iter()
                    .filter(|(p, _)| image_mime(p).is_some() && p.contains("/apps/"))
                    .max_by_key(|(p, _)| icon_size_rank(p))
            })?;
        Some(data_url(image_mime(&best.0)?, &best.1))
    }
}

fn stem_eq(path: &str, name: &str) -> bool {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().eq_ignore_ascii_case(name))
        .unwrap_or(false)
}

fn is_desktop_path(path: &str) -> bool {
    path.ends_with(".desktop") && (path.contains("share/applications/") || !path.contains('/'))
}

fn is_icon_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let image = lower.ends_with(".png") || lower.ends_with(".svg") || lower.ends_with(".xpm");
    image
        && (lower.contains("share/icons/")
            || lower.contains("share/pixmaps/")
            || !lower.contains('/')
            || lower == ".diricon")
}

pub(crate) fn read_all<R: Read>(mut r: R) -> std::io::Result<Vec<u8>> {
    let mut v = Vec::new();
    r.read_to_end(&mut v)?;
    Ok(v)
}
