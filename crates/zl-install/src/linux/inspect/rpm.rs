use std::path::Path;

use crate::linux::model::{PackageKind, PackageModel, RawDependency};
use crate::linux::util::{normalize_arch, title_case};

use super::{InspectError, PayloadScan, Result};

pub fn open(path: &Path) -> Result<::rpm::Package> {
    ::rpm::Package::open(path).map_err(|e| InspectError::Rpm(e.to_string()))
}

pub fn requires(meta: &::rpm::PackageMetadata) -> Vec<RawDependency> {
    meta.get_requires()
        .unwrap_or_default()
        .into_iter()
        .filter(|d| !d.name.starts_with("rpmlib("))
        .filter(|d| !d.flags.contains(::rpm::DependencyFlags::RPMLIB))
        .map(|d| RawDependency {
            version_req: (!d.version.is_empty()).then(|| format!("{:?} {}", d.flags, d.version)),
            name: d.name,
            alternatives: Vec::new(),
        })
        .collect()
}

pub fn inspect(path: &Path) -> Result<PackageModel> {
    let package = open(path)?;
    let meta = &package.metadata;
    let name = meta
        .get_name()
        .map_err(|e| InspectError::Rpm(e.to_string()))?
        .to_string();
    let version = {
        let v = meta.get_version().unwrap_or("0");
        match meta.get_release() {
            Ok(r) if !r.is_empty() => format!("{v}-{r}"),
            _ => v.to_string(),
        }
    };

    let mut scan = PayloadScan::default();
    for file in package
        .files()
        .map_err(|e| InspectError::Rpm(e.to_string()))?
    {
        let file = file.map_err(|e| InspectError::Rpm(e.to_string()))?;
        let p = file.metadata.path().to_string_lossy().into_owned();
        let is_dir = matches!(file.metadata.file_type(), ::rpm::FileType::Dir);
        let content = (!is_dir
            && PayloadScan::wants_content(p.trim_start_matches('/'), file.metadata.size() as u64))
        .then_some(file.content.as_slice());
        scan.visit(&p, is_dir, content);
    }

    let entry = scan.primary_desktop(&name).cloned();
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let summary = meta
        .get_summary()
        .ok()
        .map(str::to_string)
        .filter(|s| !s.is_empty());
    let description = meta
        .get_description()
        .ok()
        .map(str::to_string)
        .filter(|s| !s.is_empty());

    Ok(PackageModel {
        kind: PackageKind::Rpm,
        source_path: path.to_path_buf(),
        file_size,
        display_name: entry
            .as_ref()
            .map(|e| e.name.clone())
            .unwrap_or_else(|| title_case(&name)),
        version,
        arch: normalize_arch(meta.get_arch().unwrap_or("")),
        summary,
        description,
        homepage: meta.get_url().ok().map(str::to_string),
        license: meta.get_license().ok().map(str::to_string),
        maintainer: meta
            .get_packager()
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| meta.get_vendor().ok().filter(|s| !s.trim().is_empty()))
            .map(str::to_string),
        installed_size: meta.get_installed_size().ok(),
        dependencies: requires(meta),
        icon_data_url: scan.icon_data_url(entry.as_ref(), &name),
        desktop_entries: scan.desktop_entries.clone(),
        file_count: scan.file_count,
        top_level_paths: scan.top_level.iter().cloned().collect(),
        name,
    })
}
