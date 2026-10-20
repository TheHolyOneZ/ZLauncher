use serde::Deserialize;
use url::Url;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub url: Url,
    pub version: Option<String>,
    pub size: Option<u64>,
    pub sha256: Option<String>,
}

#[derive(Debug, Default)]
pub struct Manifest {
    pub version: Option<String>,
    pub entries: Vec<Entry>,
    pub notes: Option<String>,
    pub date: Option<String>,
}

#[derive(Deserialize)]
struct ReleasesJson {
    version: Option<String>,
    date: Option<String>,
    notes: Option<String>,
    files: Vec<ReleasesFile>,
}

#[derive(Deserialize)]
struct ReleasesFile {
    file: String,
    size: Option<u64>,
    sha256: Option<String>,
}

#[derive(Deserialize)]
struct TauriUpdater {
    version: String,
    notes: Option<String>,
    pub_date: Option<String>,
    platforms: std::collections::HashMap<String, TauriPlatform>,
}

#[derive(Deserialize)]
struct TauriPlatform {
    url: String,
}

#[derive(Deserialize)]
struct VersionJson {
    version: String,
    notes: Option<String>,
    published: Option<String>,
}

pub fn parse(body: &str, manifest_url: &Url) -> Option<Manifest> {
    if let Ok(r) = serde_json::from_str::<ReleasesJson>(body) {
        let entries = r
            .files
            .into_iter()
            .filter_map(|f| {
                Some(Entry {
                    url: manifest_url.join(&f.file).ok()?,
                    version: r.version.clone(),
                    size: f.size,
                    sha256: f.sha256.map(|s| s.to_ascii_lowercase()),
                })
            })
            .collect();
        return Some(Manifest { version: r.version, entries, notes: r.notes, date: r.date });
    }
    if let Ok(t) = serde_json::from_str::<TauriUpdater>(body) {
        let entries = t
            .platforms
            .values()
            .filter_map(|p| {
                Some(Entry { url: manifest_url.join(&p.url).ok()?, version: Some(t.version.clone()), size: None, sha256: None })
            })
            .collect();
        return Some(Manifest { version: Some(t.version), entries, notes: t.notes, date: t.pub_date });
    }
    if let Ok(v) = serde_json::from_str::<VersionJson>(body) {
        return Some(Manifest { version: Some(v.version), entries: Vec::new(), notes: v.notes, date: v.published });
    }
    None
}

pub fn parse_checksums(body: &str, list_url: &Url, single_name: Option<&str>) -> Vec<Entry> {
    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let hash = parts.next().unwrap_or("");
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let name = parts.next().map(|n| n.trim().trim_start_matches('*').trim_start_matches("./")).filter(|n| !n.is_empty());
        let Some(name) = name.or(single_name) else { continue };
        if let Ok(url) = list_url.join(name) {
            out.push(Entry { url, version: None, size: None, sha256: Some(hash.to_ascii_lowercase()) });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releases_json() {
        let u = Url::parse("https://zsync.eu/zlynstall/releases/releases.json").unwrap();
        let m = parse(r#"{"version":"0.1.0","date":"2026-09-22","files":[{"kind":"deb","file":"zlynstall_0.1.0_amd64.deb","size":4703474,"sha256":"AB"}]}"#, &u).unwrap();
        assert_eq!(m.entries[0].url.as_str(), "https://zsync.eu/zlynstall/releases/zlynstall_0.1.0_amd64.deb");
        assert_eq!(m.entries[0].sha256.as_deref(), Some("ab"));
    }

    #[test]
    fn tauri_and_version() {
        let u = Url::parse("https://zsync.eu/zgamelib/update.json").unwrap();
        let m = parse(r#"{"version":"1.2.1","pub_date":"x","platforms":{"windows-x86_64":{"signature":"s","url":"https://zsync.eu/zgamelib/app/ZGameLib_1.2.1_x64_en-US.msi"}}}"#, &u).unwrap();
        assert_eq!(m.entries.len(), 1);
        let v = parse(r#"{"version":"1.2.1","notes":"n","url":"https://zsync.eu/zgamelib/"}"#, &u).unwrap();
        assert_eq!(v.version.as_deref(), Some("1.2.1"));
        assert!(v.entries.is_empty());
    }

    #[test]
    fn checksums() {
        let u = Url::parse("https://zsync.eu/x/releases/SHA256SUMS").unwrap();
        let h = "a".repeat(64);
        let e = parse_checksums(&format!("{h}  ZBase Studio_0.2.0_amd64.deb\n{h} *b.rpm\njunk\n"), &u, None);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].url.as_str(), "https://zsync.eu/x/releases/ZBase%20Studio_0.2.0_amd64.deb");
        let one = parse_checksums(&h, &u, Some("z.msi"));
        assert_eq!(one[0].url.as_str(), "https://zsync.eu/x/releases/z.msi");
    }
}
