use std::collections::BTreeSet;
use std::sync::OnceLock;

use regex::Regex;
use scraper::{Html, Selector};
use url::Url;

use crate::classify::classify;

#[derive(Debug, Default)]
pub struct Scan {
    pub files: BTreeSet<Url>,
    pub aliases: BTreeSet<Url>,
    pub checksum_lists: BTreeSet<Url>,
    pub manifests: BTreeSet<Url>,
    pub script_names: BTreeSet<String>,
    pub scripts: BTreeSet<Url>,
    pub subpages: BTreeSet<Url>,
    pub versions: BTreeSet<String>,
}

pub fn last_segment(url: &Url) -> String {
    let seg = url.path_segments().and_then(|mut s| s.next_back().map(str::to_string)).unwrap_or_default();
    percent_encoding::percent_decode_str(&seg).decode_utf8_lossy().into_owned()
}

fn is_checksum_name(lower: &str) -> bool {
    lower.starts_with("sha256sums") || lower == "checksums.txt" || lower.ends_with(".sha256")
}

fn release_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)["'`]([^"'`\s<>(){}]{1,200}\.(?:deb|rpm|appimage|msi|exe|dmg|apk|ipa|xpi|zip|tar\.gz|pkg\.tar\.zst))["'`]"#)
            .unwrap()
    })
}

fn json_ref_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"["'`]([A-Za-z0-9_./-]{1,120}\.json)["'`]"#).unwrap())
}

fn version_ld_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#""softwareVersion"\s*:\s*"([^"]+)""#).unwrap())
}

fn same_site(a: &Url, b: &Url) -> bool {
    a.host_str() == b.host_str()
}

pub fn scan_script(text: &str, base: &Url, out: &mut Scan) {
    for c in release_name_re().captures_iter(text) {
        let name = &c[1];
        if name.contains("://") {
            if let Ok(u) = Url::parse(name) {
                out.files.insert(u);
            }
        } else {
            out.script_names.insert(name.to_string());
        }
    }
    for c in json_ref_re().captures_iter(text) {
        let name = &c[1];
        let lower = name.to_ascii_lowercase();
        if ["release", "latest", "version", "update"].iter().any(|k| lower.contains(k)) {
            if let Ok(u) = base.join(name) {
                if same_site(&u, base) {
                    out.manifests.insert(u);
                }
            }
        }
    }
}

pub fn scan_html(body: &str, page: &Url) -> Scan {
    let doc = Html::parse_document(body);
    let mut out = Scan::default();
    let base = Selector::parse("base[href]")
        .ok()
        .and_then(|s| doc.select(&s).next().and_then(|b| b.value().attr("href")).and_then(|h| page.join(h).ok()))
        .unwrap_or_else(|| page.clone());

    let a_sel = Selector::parse("a[href], [data-file], [data-href], [data-url]").unwrap();
    for el in doc.select(&a_sel) {
        for attr in ["href", "data-file", "data-href", "data-url"] {
            let Some(raw) = el.value().attr(attr) else { continue };
            let raw = raw.trim();
            if raw.is_empty() || raw.starts_with('#') || raw.starts_with("mailto:") || raw.starts_with("javascript:") {
                continue;
            }
            if attr == "data-file" && !raw.contains('/') {
                if classify(raw).is_some() {
                    out.script_names.insert(raw.to_string());
                }
                continue;
            }
            let Ok(mut u) = base.join(raw) else { continue };
            u.set_fragment(None);
            let name = last_segment(&u);
            let lower = name.to_ascii_lowercase();
            if is_checksum_name(&lower) {
                out.checksum_lists.insert(u);
            } else if u.path().contains("/latest/") && same_site(&u, page) {
                out.aliases.insert(u);
            } else if classify(&name).is_some() {
                out.files.insert(u);
            } else if attr == "href" && same_site(&u, page) && u.path() != page.path() {
                let path = u.path().to_ascii_lowercase();
                let text = el.text().collect::<String>().to_ascii_lowercase();
                let looks = ["launcher", "download"].iter().any(|k| path.contains(k) || text.contains(k));
                if looks && u.path().starts_with(page.path()) && !lower.contains('.') {
                    out.subpages.insert(u);
                }
            }
        }
    }

    let script_sel = Selector::parse("script").unwrap();
    for s in doc.select(&script_sel) {
        if let Some(src) = s.value().attr("src") {
            if let Ok(u) = base.join(src) {
                if same_site(&u, page) && u.path().ends_with(".js") {
                    out.scripts.insert(u);
                }
            }
            continue;
        }
        let text: String = s.text().collect();
        if s.value().attr("type") == Some("application/ld+json") {
            for c in version_ld_re().captures_iter(&text) {
                out.versions.insert(c[1].to_string());
            }
        }
        scan_script(&text, &base, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_links_aliases_and_scripts() {
        let page = Url::parse("https://zsync.eu/zdemo/").unwrap();
        let html = r##"<html><body>
            <a class="dl-file" href="releases/ZDemo_1.0.0_amd64.deb">deb</a>
            <a href="./releases/ZBase%20Studio_0.2.0_x64-setup.exe" download>exe</a>
            <a href="releases/latest/linux-rpm">rpm</a>
            <a href="releases/SHA256SUMS.txt">sums</a>
            <a href="https://zsync.eu/zdemo/launcher/">Get the launcher</a>
            <a href="https://zsync.eu/other/">Download other</a>
            <a href="#download">jump</a>
            <a data-dl="appimage">js-filled</a>
            <script src="assets/app.js"></script>
            <script type="application/ld+json">{"softwareVersion": "1.0.0"}</script>
            <script>const RELEASE = { files: { appimage: { name: "ZDemo_1.0.0_amd64.AppImage" } } };
              fetch('releases/releases.json'); fetch('i18n/en.json');</script>
        </body></html>"##;
        let s = scan_html(html, &page);
        assert_eq!(s.files.len(), 2);
        assert!(s.files.iter().any(|u| u.as_str().ends_with("ZBase%20Studio_0.2.0_x64-setup.exe")));
        assert_eq!(s.aliases.len(), 1);
        assert_eq!(s.checksum_lists.len(), 1);
        assert_eq!(s.subpages.iter().map(Url::as_str).collect::<Vec<_>>(), ["https://zsync.eu/zdemo/launcher/"]);
        assert!(s.script_names.contains("ZDemo_1.0.0_amd64.AppImage"));
        assert_eq!(s.manifests.iter().map(Url::as_str).collect::<Vec<_>>(), ["https://zsync.eu/zdemo/releases/releases.json"]);
        assert!(s.scripts.iter().any(|u| u.as_str() == "https://zsync.eu/zdemo/assets/app.js"));
        assert!(s.versions.contains("1.0.0"));
    }
}
