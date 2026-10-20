use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use backhand::{FilesystemReader, InnerNode};

use crate::linux::model::{PackageKind, PackageModel};
use crate::linux::util::{slugify, title_case};

use super::{InspectError, PayloadScan, Result};

const SQUASHFS_MAGIC: &[u8; 4] = b"hsqs";

#[derive(Debug, Clone, Copy)]
pub struct ElfInfo {
    pub squashfs_offset: u64,
    pub arch: &'static str,
}

pub fn elf_info(path: &Path) -> Result<ElfInfo> {
    let mut f =
        std::fs::File::open(path).map_err(|e| InspectError::Io(path.display().to_string(), e))?;
    let mut hdr = [0u8; 64];
    f.read_exact(&mut hdr)
        .map_err(|_| InspectError::AppImage("file too small for an ELF header".into()))?;
    if &hdr[0..4] != b"\x7fELF" {
        return Err(InspectError::AppImage("not an ELF file".into()));
    }
    let is64 = hdr[4] == 2;
    let le = hdr[5] == 1;
    let u16at = |o: usize| -> u16 {
        let b = [hdr[o], hdr[o + 1]];
        if le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        }
    };
    let machine = u16at(18);
    let arch = match machine {
        0x3E => "x86_64",
        0xB7 => "aarch64",
        0x03 => "x86",
        0x28 => "arm",
        _ => "unknown",
    };
    let (shoff, shentsize, shnum) = if is64 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&hdr[0x28..0x30]);
        let shoff = if le {
            u64::from_le_bytes(b)
        } else {
            u64::from_be_bytes(b)
        };
        (shoff, u16at(0x3A) as u64, u16at(0x3C) as u64)
    } else {
        let mut b = [0u8; 4];
        b.copy_from_slice(&hdr[0x20..0x24]);
        let shoff = if le {
            u32::from_le_bytes(b) as u64
        } else {
            u32::from_be_bytes(b) as u64
        };
        (shoff, u16at(0x2E) as u64, u16at(0x30) as u64)
    };
    let mut offset = shoff + shentsize * shnum;

    let mut magic = [0u8; 4];
    let ok = f.seek(SeekFrom::Start(offset)).is_ok()
        && f.read_exact(&mut magic).is_ok()
        && &magic == SQUASHFS_MAGIC;
    if !ok {
        offset = scan_for_magic(&mut f)?;
    }
    Ok(ElfInfo {
        squashfs_offset: offset,
        arch,
    })
}

fn scan_for_magic(f: &mut std::fs::File) -> Result<u64> {
    f.seek(SeekFrom::Start(0))
        .map_err(|e| InspectError::AppImage(e.to_string()))?;
    let mut buf = vec![0u8; 4 * 1024 * 1024];
    let n = f
        .read(&mut buf)
        .map_err(|e| InspectError::AppImage(e.to_string()))?;

    (1024..n.saturating_sub(4))
        .find(|&i| &buf[i..i + 4] == SQUASHFS_MAGIC)
        .map(|i| i as u64)
        .ok_or_else(|| {
            InspectError::AppImage(
                "no squashfs image found (type 1 AppImages are not supported)".into(),
            )
        })
}

fn read_file(fs: &FilesystemReader, path: &Path) -> Option<Vec<u8>> {
    let node = fs.files().find(|n| n.fullpath == path)?;
    match &node.inner {
        InnerNode::File(file) => {
            let mut out = Vec::new();
            fs.file(file).reader().read_to_end(&mut out).ok()?;
            Some(out)
        }
        InnerNode::Symlink(link) => {
            let target = resolve_link(path, &link.link);
            read_file(fs, &target)
        }
        _ => None,
    }
}

fn resolve_link(from: &Path, link: &Path) -> PathBuf {
    if link.is_absolute() {
        link.to_path_buf()
    } else {
        let mut base = from
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("/"));
        for comp in link.components() {
            match comp {
                std::path::Component::ParentDir => {
                    base.pop();
                }
                std::path::Component::Normal(s) => base.push(s),
                _ => {}
            }
        }
        base
    }
}

pub struct AppImageContents {
    pub scan: PayloadScan,
    pub elf: ElfInfo,

    pub version_hint: Option<String>,
}

pub fn read_contents(path: &Path) -> Result<AppImageContents> {
    let elf = elf_info(path)?;
    let file =
        std::fs::File::open(path).map_err(|e| InspectError::Io(path.display().to_string(), e))?;
    let fs = FilesystemReader::from_reader_with_offset(BufReader::new(file), elf.squashfs_offset)
        .map_err(|e| InspectError::AppImage(format!("squashfs: {e}")))?;

    let mut scan = PayloadScan::default();
    let mut version_hint = None;

    for node in fs.files() {
        let full = node.fullpath.to_string_lossy().into_owned();
        let rel = full.trim_start_matches('/');
        if rel.is_empty() {
            continue;
        }
        match &node.inner {
            InnerNode::Dir(_) => scan.visit(rel, true, None),
            InnerNode::File(file) => {
                let size = file.file_len() as u64;
                let wants = PayloadScan::wants_content(rel, size) || rel == ".DirIcon";
                let content = if wants {
                    let mut out = Vec::new();
                    fs.file(file).reader().read_to_end(&mut out).ok();
                    Some(out)
                } else {
                    None
                };
                if rel.ends_with(".desktop") && !rel.contains('/') {
                    if let Some(c) = &content {
                        version_hint = String::from_utf8_lossy(c).lines().find_map(|l| {
                            l.strip_prefix("X-AppImage-Version=")
                                .map(|v| v.trim().to_string())
                        });
                    }
                }
                scan.visit(rel, false, content.as_deref());
            }
            InnerNode::Symlink(link) => {
                if rel == ".DirIcon" || PayloadScan::wants_content(rel, 0) {
                    let target = resolve_link(&node.fullpath, &link.link);
                    if let Some(bytes) = read_file(&fs, &target) {
                        let target_name = target.to_string_lossy().into_owned();

                        scan.visit(target_name.trim_start_matches('/'), false, Some(&bytes));
                        if rel == ".DirIcon" {
                            scan.icons.push((
                                format!(
                                    "diricon/{}",
                                    target
                                        .file_name()
                                        .map(|s| s.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| "icon.png".into())
                                ),
                                bytes,
                            ));
                        }
                    }
                }
                scan.visit(rel, false, None);
            }
            _ => {}
        }
    }

    Ok(AppImageContents {
        scan,
        elf,
        version_hint,
    })
}

fn version_from_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();

    stem.split(['-', '_'])
        .find(|seg| {
            seg.chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
                && seg.contains('.')
        })
        .map(|s| s.to_string())
}

pub fn inspect(path: &Path) -> Result<PackageModel> {
    let contents = read_contents(path)?;
    let scan = &contents.scan;
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    let entry = scan.primary_desktop("").cloned();
    let display_name = entry.as_ref().map(|e| e.name.clone()).unwrap_or_else(|| {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        title_case(stem.split(['-', '_']).next().unwrap_or(&stem))
    });
    let name = slugify(&display_name);

    Ok(PackageModel {
        kind: PackageKind::AppImage,
        source_path: path.to_path_buf(),
        file_size,
        version: contents
            .version_hint
            .clone()
            .or_else(|| version_from_filename(path))
            .unwrap_or_else(|| "unknown".into()),
        arch: contents.elf.arch.to_string(),
        summary: entry.as_ref().and_then(|e| e.comment.clone()),
        description: None,
        homepage: None,
        license: None,
        maintainer: None,
        installed_size: None,
        dependencies: Vec::new(),
        icon_data_url: scan.icon_data_url(entry.as_ref(), &name),
        desktop_entries: scan.desktop_entries.clone(),
        file_count: scan.file_count,
        top_level_paths: scan.top_level.iter().cloned().collect(),
        display_name,
        name,
    })
}
