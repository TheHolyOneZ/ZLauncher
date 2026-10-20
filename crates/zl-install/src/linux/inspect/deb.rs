use std::collections::HashMap;
use std::io::{self, Read};
use std::path::Path;

use crate::linux::model::{PackageKind, PackageModel, RawDependency};
use crate::linux::util::{normalize_arch, title_case};

use super::{read_all, InspectError, PayloadScan, Result};

#[derive(Debug, Default, Clone)]
pub struct Control {
    pub fields: HashMap<String, String>,
}

impl Control {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    pub fn parse(text: &str) -> Control {
        let mut fields: HashMap<String, String> = HashMap::new();
        let mut current: Option<String> = None;
        for line in text.lines() {
            if line.starts_with(' ') || line.starts_with('\t') {
                if let Some(key) = &current {
                    let entry = fields.entry(key.clone()).or_default();
                    entry.push('\n');
                    let cont = line.trim_start();
                    entry.push_str(if cont == "." { "" } else { cont });
                }
                continue;
            }
            if let Some((k, v)) = line.split_once(':') {
                let k = k.trim().to_string();
                fields.insert(k.clone(), v.trim().to_string());
                current = Some(k);
            }
        }
        Control { fields }
    }

    pub fn dependencies(&self) -> Vec<RawDependency> {
        let mut out = Vec::new();
        for key in ["Pre-Depends", "Depends"] {
            if let Some(v) = self.get(key) {
                out.extend(parse_depends(v));
            }
        }
        out
    }

    pub fn summary_and_description(&self) -> (Option<String>, Option<String>) {
        match self.get("Description") {
            Some(d) => {
                let mut lines = d.splitn(2, '\n');
                let summary = lines
                    .next()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                let desc = lines
                    .next()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                (summary, desc)
            }
            None => (None, None),
        }
    }
}

pub fn parse_depends(field: &str) -> Vec<RawDependency> {
    field
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|clause| {
            let alts: Vec<(String, Option<String>)> = clause
                .split('|')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|alt| {
                    let (name, ver) = match alt.split_once('(') {
                        Some((n, v)) => {
                            (n.trim(), Some(v.trim_end_matches(')').trim().to_string()))
                        }
                        None => (alt, None),
                    };

                    let name = name.split(':').next().unwrap_or(name).trim().to_string();
                    (name, ver)
                })
                .collect();
            let (name, version_req) = alts.first().cloned().unwrap_or_default();
            RawDependency {
                name,
                version_req,
                alternatives: alts.iter().map(|(n, _)| n.clone()).collect(),
            }
        })
        .collect()
}

pub(crate) fn decompress<'a>(name: &str, reader: Box<dyn Read + 'a>) -> Result<Box<dyn Read + 'a>> {
    Ok(if name.ends_with(".gz") {
        Box::new(flate2::read::GzDecoder::new(reader))
    } else if name.ends_with(".xz") {
        Box::new(liblzma::read::XzDecoder::new_multi_decoder(reader))
    } else if name.ends_with(".zst") {
        Box::new(
            zstd::stream::read::Decoder::new(reader)
                .map_err(|e| InspectError::Deb(e.to_string()))?,
        )
    } else if name.ends_with(".tar") {
        reader
    } else {
        return Err(InspectError::Deb(format!(
            "unsupported member compression: {name}"
        )));
    })
}

pub(crate) fn walk_members<F>(path: &Path, mut on_member: F) -> Result<()>
where
    F: FnMut(&str, Box<dyn Read + '_>) -> Result<()>,
{
    let file =
        std::fs::File::open(path).map_err(|e| InspectError::Io(path.display().to_string(), e))?;
    let mut archive = ar::Archive::new(io::BufReader::new(file));
    let mut saw_control = false;
    while let Some(entry) = archive.next_entry() {
        let entry = entry.map_err(|e| InspectError::Deb(e.to_string()))?;
        let name = String::from_utf8_lossy(entry.header().identifier())
            .trim()
            .to_string();
        if name.starts_with("control.tar") || name.starts_with("data.tar") {
            if name.starts_with("control.tar") {
                saw_control = true;
            }
            let reader = decompress(&name, Box::new(entry))?;
            on_member(&name, reader)?;
        }
    }
    if !saw_control {
        return Err(InspectError::Deb("no control.tar member".into()));
    }
    Ok(())
}

pub fn inspect(path: &Path) -> Result<PackageModel> {
    let mut control: Option<Control> = None;
    let mut scan = PayloadScan::default();

    walk_members(path, |name, reader| {
        let mut tar = tar::Archive::new(reader);
        let entries = tar
            .entries()
            .map_err(|e| InspectError::Deb(e.to_string()))?;
        if name.starts_with("control.tar") {
            for entry in entries {
                let mut entry = entry.map_err(|e| InspectError::Deb(e.to_string()))?;
                let p = entry
                    .path()
                    .map_err(|e| InspectError::Deb(e.to_string()))?
                    .to_string_lossy()
                    .into_owned();
                if p.trim_start_matches("./") == "control" {
                    let text = String::from_utf8_lossy(
                        &read_all(&mut entry).map_err(|e| InspectError::Deb(e.to_string()))?,
                    )
                    .into_owned();
                    control = Some(Control::parse(&text));
                }
            }
        } else {
            for entry in entries {
                let mut entry = entry.map_err(|e| InspectError::Deb(e.to_string()))?;
                let p = entry
                    .path()
                    .map_err(|e| InspectError::Deb(e.to_string()))?
                    .to_string_lossy()
                    .into_owned();
                let is_dir = entry.header().entry_type().is_dir();
                let size = entry.header().size().unwrap_or(0);
                let content =
                    if !is_dir && PayloadScan::wants_content(p.trim_start_matches("./"), size) {
                        Some(read_all(&mut entry).map_err(|e| InspectError::Deb(e.to_string()))?)
                    } else {
                        None
                    };
                scan.visit(&p, is_dir, content.as_deref());
            }
        }
        Ok(())
    })?;

    let control = control.ok_or_else(|| InspectError::Deb("control file missing".into()))?;
    let name = control.get("Package").unwrap_or("package").to_string();
    let entry = scan.primary_desktop(&name).cloned();
    let (summary, description) = control.summary_and_description();
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    Ok(PackageModel {
        kind: PackageKind::Deb,
        source_path: path.to_path_buf(),
        file_size,
        display_name: entry
            .as_ref()
            .map(|e| e.name.clone())
            .unwrap_or_else(|| title_case(&name)),
        version: control.get("Version").unwrap_or("0").to_string(),
        arch: normalize_arch(control.get("Architecture").unwrap_or("")),
        summary,
        description,
        homepage: control.get("Homepage").map(str::to_string),
        license: None,
        maintainer: control.get("Maintainer").map(str::to_string),
        installed_size: control
            .get("Installed-Size")
            .and_then(|s| s.parse::<u64>().ok())
            .map(|kb| kb * 1024),
        dependencies: control.dependencies(),
        icon_data_url: scan.icon_data_url(entry.as_ref(), &name),
        desktop_entries: scan.desktop_entries.clone(),
        file_count: scan.file_count,
        top_level_paths: scan.top_level.iter().cloned().collect(),
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_control_with_multiline_description() {
        let c = Control::parse(
            "Package: discord\nVersion: 0.0.99\nArchitecture: amd64\nDepends: libc6 (>= 2.17), libgtk-3-0 | libgtk-3-0t64, libnotify4\nInstalled-Size: 300000\nDescription: All-in-one voice and text chat\n Long text line one.\n .\n Line two.\n",
        );
        assert_eq!(c.get("Package"), Some("discord"));
        let (s, d) = c.summary_and_description();
        assert_eq!(s.as_deref(), Some("All-in-one voice and text chat"));
        assert_eq!(d.as_deref(), Some("Long text line one.\n\nLine two."));
        let deps = c.dependencies();
        assert_eq!(deps.len(), 3);
        assert_eq!(deps[0].name, "libc6");
        assert_eq!(deps[0].version_req.as_deref(), Some(">= 2.17"));
        assert_eq!(deps[1].alternatives, vec!["libgtk-3-0", "libgtk-3-0t64"]);
    }

    #[test]
    fn depends_strips_arch_qualifiers() {
        let d = parse_depends("libc6:amd64 (>= 2.34), foo:any");
        assert_eq!(d[0].name, "libc6");
        assert_eq!(d[1].name, "foo");
    }
}
