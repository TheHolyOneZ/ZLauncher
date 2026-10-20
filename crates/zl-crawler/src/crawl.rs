use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use futures::stream::{self, StreamExt};
use url::Url;
use zl_catalog::{parse_github, Project};

use crate::classify::{classify, cmp_versions, Classified};
use crate::fetch::{resolve, Fetcher};
use crate::manifest::{self, Entry};
use crate::model::{Arch, Artifact, CrawlReport, Kind, Os, Source};
use crate::overrides::Overrides;
use crate::scan::{last_segment, scan_html, scan_script};

pub const CRAWL_HOSTS: &[&str] = &["zsync.eu", "zlogic.eu"];

const MAX_SCRIPTS: usize = 4;
const MAX_SUBPAGES: usize = 3;

pub struct Crawler {
    fetcher: Arc<dyn Fetcher>,
    overrides: Overrides,
    github_api: Url,
    pub verify: bool,
    pub use_github: bool,
}

#[derive(Debug, Clone)]
struct Found {
    url: Url,
    version: Option<String>,
    size: Option<u64>,
    sha256: Option<String>,
    source: Source,
}

#[derive(Default)]
struct State {
    found: BTreeMap<String, Found>,
    hashes: HashMap<String, String>,
    version_hint: Option<String>,
    release: Option<(Option<String>, Option<String>, Option<String>)>,
    notes: Vec<String>,
    visited: BTreeSet<String>,
}

impl State {
    fn add(&mut self, e: Entry, source: Source) {
        let key = e.url.to_string();
        match self.found.get_mut(&key) {
            Some(f) => {
                f.version = f.version.take().or(e.version);
                f.size = f.size.or(e.size);
                f.sha256 = f.sha256.take().or(e.sha256);
                f.source = f.source.min(source);
            }
            None => {
                self.found.insert(
                    key,
                    Found { url: e.url, version: e.version, size: e.size, sha256: e.sha256, source },
                );
            }
        }
    }

    fn hint(&mut self, v: Option<String>) {
        if let Some(v) = v {
            if self.version_hint.as_deref().is_none_or(|h| cmp_versions(&v, h).is_gt()) {
                self.version_hint = Some(v);
            }
        }
    }
}

fn entry(url: Url) -> Entry {
    Entry { url, version: None, size: None, sha256: None }
}

fn dir_of(u: &Url) -> Url {
    let mut d = u.clone();
    d.set_query(None);
    d.set_fragment(None);
    if !d.path().ends_with('/') {
        let p = d.path().rsplit_once('/').map(|(a, _)| format!("{a}/")).unwrap_or_else(|| "/".into());
        d.set_path(&p);
    }
    d
}

fn recase(dead: &str, sibling: &str) -> Option<String> {
    let product_len = |s: &str| {
        let b = s.as_bytes();
        (0..b.len()).find(|&i| b[i] == b'_' || (b[i] == b'-' && b.get(i + 1).is_some_and(u8::is_ascii_digit)))
    };
    let (d, s) = (product_len(dead)?, product_len(sibling)?);
    if d != s || d < 2 || !dead[..d].eq_ignore_ascii_case(&sibling[..s]) || dead[..d] == sibling[..s] {
        return None;
    }
    Some(format!("{}{}", &sibling[..s], &dead[d..]))
}

async fn case_fix(fetcher: &dyn Fetcher, url: &Url, name: &str) -> Option<(Url, Option<u64>)> {
    let dir = dir_of(url);
    let listing = fetcher.get(&dir).await.ok().filter(|p| p.ok() && p.body.contains("Index of"))?;
    let real = scan_html(&listing.body, &listing.url)
        .files
        .into_iter()
        .find(|u| last_segment(u).eq_ignore_ascii_case(name) && u != url)?;
    let head = fetcher.head(&real).await.ok().filter(|h| h.ok())?;
    Some((real, head.size))
}

fn crawlable(u: &Url) -> bool {
    u.host_str().is_some_and(|h| CRAWL_HOSTS.contains(&h.trim_start_matches("www.")))
}

impl Crawler {
    pub fn new(fetcher: Arc<dyn Fetcher>) -> Self {
        Self {
            fetcher,
            overrides: Overrides::bundled(),
            github_api: Url::parse("https://api.github.com/").unwrap(),
            verify: true,
            use_github: true,
        }
    }

    pub fn with_overrides(mut self, o: Overrides) -> Self {
        self.overrides = o;
        self
    }

    pub async fn crawl_all(&self, projects: &[Project], concurrency: usize) -> Vec<CrawlReport> {
        let mut out: Vec<(usize, CrawlReport)> = stream::iter(projects.iter().enumerate())
            .map(|(i, p)| async move { (i, self.crawl(p).await) })
            .buffer_unordered(concurrency.max(1))
            .collect()
            .await;
        out.sort_by_key(|(i, _)| *i);
        out.into_iter().map(|(_, r)| r).collect()
    }

    pub async fn crawl(&self, p: &Project) -> CrawlReport {
        let ov = self.overrides.get(&p.id);
        let mut st = State::default();
        if ov.is_some_and(|o| o.skip) {
            st.notes.push("no downloads (override)".into());
            return self.finish(p, st).await;
        }

        let mut pages: Vec<Url> = p.url.iter().filter_map(|u| Url::parse(u).ok()).collect();
        if let Some(o) = ov {
            pages.extend(o.extra_pages.iter().filter_map(|u| Url::parse(u).ok()));
        }
        let mut repo = p.github_repo();
        for page in &pages {
            if let Some(r) = parse_github(page.as_str()) {
                repo = repo.or(Some(r));
                continue;
            }
            if !crawlable(page) {
                st.notes.push(format!("{page} is not a zsync/zlogic page"));
                continue;
            }
            self.crawl_page(page, 0, ov.map(|o| o.manifests.as_slice()).unwrap_or(&[]), &mut st).await;
        }

        for l in &p.links {
            if let Ok(u) = Url::parse(&l.url) {
                if classify(&last_segment(&u)).is_some() {
                    st.add(entry(u), Source::Link);
                }
            }
        }

        if st.found.is_empty() && self.use_github {
            if let Some((owner, name)) = repo {
                let latest = crate::github::latest(self.fetcher.as_ref(), &self.github_api, &owner, &name).await;
                if latest.entries.is_empty() {
                    st.notes.push(format!("github {owner}/{name}: no release assets"));
                }
                if latest.notes.is_some() || latest.date.is_some() {
                    let v = latest.entries.first().and_then(|e| e.version.clone());
                    st.release = Some((v, latest.notes, latest.date));
                }
                for e in latest.entries {
                    st.add(e, Source::Github);
                }
            }
        }
        self.finish(p, st).await
    }

    async fn get_ok(&self, u: &Url) -> Option<crate::fetch::Page> {
        match self.fetcher.get(u).await {
            Ok(p) if p.ok() => Some(p),
            Ok(_) => None,
            Err(e) => {
                tracing::debug!("GET {u}: {e}");
                None
            }
        }
    }

    fn crawl_page<'a>(
        &'a self,
        page_url: &'a Url,
        depth: u8,
        extra_manifests: &'a [String],
        st: &'a mut State,
    ) -> futures::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            if !st.visited.insert(page_url.to_string()) {
                return;
            }
            let Some(page) = self.get_ok(page_url).await else {
                st.notes.push(format!("could not load {page_url}"));
                return;
            };
            let url = page.url.clone();
            let page_dir = dir_of(&url);
            let mut scan = scan_html(&page.body, &url);

            for s in scan.scripts.clone().into_iter().take(MAX_SCRIPTS) {
                if let Some(js) = self.get_ok(&s).await {
                    scan_script(&js.body, &url, &mut scan);
                }
            }
            for v in &scan.versions {
                st.hint(Some(v.clone()));
            }

            let mut manifests = scan.manifests.clone();
            for rel in ["releases/releases.json", "latest.json", "update.json"]
                .into_iter()
                .chain(extra_manifests.iter().map(String::as_str))
            {
                if let Ok(u) = page_dir.join(rel) {
                    manifests.insert(u);
                }
            }
            for m in &manifests {
                let Some(body) = self.get_ok(m).await else { continue };
                if let Some(parsed) = manifest::parse(&body.body, &body.url) {
                    if parsed.notes.is_some() || parsed.date.is_some() {
                        st.release = Some((parsed.version.clone(), parsed.notes.clone(), parsed.date.clone()));
                    }
                    st.hint(parsed.version);
                    for e in parsed.entries {
                        st.add(e, Source::Manifest);
                    }
                }
            }

            for f in &scan.files {
                st.add(entry(f.clone()), Source::Link);
            }
            for a in &scan.aliases {
                match resolve(self.fetcher.as_ref(), a, 4).await {
                    Ok((target, head)) if head.ok() && classify(&last_segment(&target)).is_some() => {
                        st.add(Entry { url: target, version: None, size: head.size, sha256: None }, Source::Alias);
                    }
                    Ok(_) => {}
                    Err(e) => st.notes.push(format!("alias {a}: {e}")),
                }
            }

            let mut dirs: BTreeSet<Url> = st.found.values().map(|f| dir_of(&f.url)).filter(crawlable).collect();
            for rel in ["releases/", "release/"] {
                if let Ok(u) = page_dir.join(rel) {
                    dirs.insert(u);
                }
            }

            let mut lists = scan.checksum_lists.clone();
            let linked_dirs: BTreeSet<Url> = lists.iter().map(dir_of).collect();
            for d in &dirs {
                if linked_dirs.contains(d) {
                    continue;
                }
                for name in ["SHA256SUMS", "SHA256SUMS.txt"] {
                    if let Ok(u) = d.join(name) {
                        lists.insert(u);
                    }
                }
            }
            for l in &lists {
                let Some(body) = self.get_ok(l).await else { continue };
                let name = last_segment(l);
                let single = name.strip_suffix(".sha256").filter(|n| !n.to_ascii_lowercase().starts_with("sha256sums"));
                for e in manifest::parse_checksums(&body.body, l, single) {
                    if let Some(h) = &e.sha256 {
                        st.hashes.insert(e.url.to_string(), h.clone());
                    }
                    st.add(e, Source::Checksums);
                }
            }

            for name in &scan.script_names {
                let candidates: Vec<Url> = if name.contains('/') {
                    url.join(name).ok().into_iter().collect()
                } else {
                    dirs.iter().chain(std::iter::once(&page_dir)).filter_map(|d| d.join(name).ok()).collect()
                };
                if candidates.iter().any(|c| st.found.contains_key(c.as_str())) {
                    continue;
                }
                for c in candidates {
                    if let Ok((target, head)) = resolve(self.fetcher.as_ref(), &c, 3).await {
                        if head.ok() {
                            st.add(Entry { url: target, version: None, size: head.size, sha256: None }, Source::Script);
                            break;
                        }
                    }
                }
            }

            if st.found.is_empty() && depth == 0 {
                for sub in scan.subpages.clone().into_iter().take(MAX_SUBPAGES) {
                    self.crawl_page(&sub, depth + 1, &[], st).await;
                }
            }

            if st.found.is_empty() {
                for rel in ["releases/", "release/"] {
                    let Ok(u) = page_dir.join(rel) else { continue };
                    let Some(listing) = self.get_ok(&u).await else { continue };
                    if !listing.body.contains("Index of") {
                        continue;
                    }
                    for f in scan_html(&listing.body, &listing.url).files {
                        st.add(entry(f), Source::Listing);
                    }
                }
            }
        })
    }

    async fn finish(&self, p: &Project, mut st: State) -> CrawlReport {
        let mut items: Vec<(String, Found, Classified)> = Vec::new();
        for (key, mut f) in std::mem::take(&mut st.found) {
            let name = last_segment(&f.url);
            let Some(c) = classify(&name) else { continue };
            if f.sha256.is_none() {
                f.sha256 = st.hashes.get(&key).cloned();
            }
            items.push((name, f, c));
        }

        if self.verify {
            let fetcher = self.fetcher.clone();
            let checked: Vec<((String, Found, Classified), Option<String>)> = stream::iter(items)
                .map(|(name, mut f, c)| {
                    let fetcher = fetcher.clone();
                    async move {
                        if f.size.is_some() && f.source <= Source::Alias {
                            return ((name, f, c), None);
                        }
                        match resolve(fetcher.as_ref(), &f.url, 4).await {
                            Ok((_, h)) if h.ok() => {
                                f.size = f.size.or(h.size);
                                ((name, f, c), None)
                            }
                            Ok((_, h)) => {
                                if f.source == Source::Link {
                                    if let Some((real, size)) = case_fix(fetcher.as_ref(), &f.url, &name).await {
                                        tracing::info!("link with wrong case: {} -> {}", f.url, real);
                                        f.url = real;
                                        f.size = size;
                                        let fixed = last_segment(&f.url);
                                        return ((fixed, f, c), None);
                                    }
                                }
                                let note = format!("dead link ({}): {}", h.status, f.url);
                                ((name, f, c), Some(note))
                            }
                            Err(_) => ((name, f, c), None),
                        }
                    }
                })
                .buffer_unordered(4)
                .collect()
                .await;
            items = Vec::new();
            let mut dead = Vec::new();
            for (item, note) in checked {
                match note {
                    Some(_) if item.1.source == Source::Checksums => {}
                    Some(n) => dead.push((item, n)),
                    None => items.push(item),
                }
            }
            for ((name, mut f, c), note) in dead {
                let dir = dir_of(&f.url);
                let fixed = items
                    .iter()
                    .filter(|(_, s, _)| dir_of(&s.url) == dir)
                    .filter_map(|(sname, _, _)| recase(&name, sname))
                    .find(|cand| cand != &name);
                let mut ok = false;
                if let Some(cand) = fixed.and_then(|n| dir.join(&n).ok()) {
                    if let Ok(h) = self.fetcher.head(&cand).await {
                        if h.ok() {
                            st.notes.push(format!("link with wrong case fixed: {} -> {}", f.url, cand));
                            f.url = cand;
                            f.size = h.size;
                            items.push((last_segment(&f.url), f.clone(), c.clone()));
                            ok = true;
                        }
                    }
                }
                if !ok {
                    st.notes.push(note);
                }
            }
        }

        let mut by_name: BTreeMap<String, (Found, Classified)> = BTreeMap::new();
        for (name, f, c) in items {
            match by_name.get(&name) {
                Some((old, _)) if old.source <= f.source => {}
                _ => {
                    by_name.insert(name, (f, c));
                }
            }
        }

        let mut artifacts: Vec<Artifact> = by_name
            .into_iter()
            .map(|(name, (f, c))| Artifact {
                project_id: p.id.clone(),
                url: f.url.to_string(),
                filename: name,
                os: c.os,
                arch: c.arch,
                kind: c.kind,
                version: c.version.or(f.version).or_else(|| st.version_hint.clone()),
                size: f.size,
                sha256: f.sha256,
                source: f.source,
            })
            .collect();

        let mut newest: HashMap<(Os, Kind, Option<Arch>), String> = HashMap::new();
        for a in &artifacts {
            if let Some(v) = &a.version {
                let k = (a.os, a.kind, a.arch);
                if newest.get(&k).is_none_or(|n| cmp_versions(v, n).is_gt()) {
                    newest.insert(k, v.clone());
                }
            }
        }
        artifacts.retain(|a| match (&a.version, newest.get(&(a.os, a.kind, a.arch))) {
            (Some(v), Some(n)) => cmp_versions(v, n).is_ge(),
            _ => true,
        });

        artifacts.sort_by(|a, b| (a.os, a.kind, a.arch, &a.filename).cmp(&(b.os, b.kind, b.arch, &b.filename)));
        let version = artifacts
            .iter()
            .filter_map(|a| a.version.clone())
            .max_by(|a, b| cmp_versions(a, b))
            .or(st.version_hint);
        let (release_notes, released) = match st.release {
            Some((v, n, d)) if v.is_none() || v == version => (n, d),
            _ => (None, None),
        };
        CrawlReport { project_id: p.id.clone(), artifacts, version, notes: st.notes, release_notes, released }
    }
}

#[cfg(test)]
mod tests {
    use super::recase;

    #[test]
    fn recases_from_sibling() {
        assert_eq!(recase("z-stego_0.1.0_amd64.deb", "Z-Stego_0.1.0_x64.dmg").as_deref(), Some("Z-Stego_0.1.0_amd64.deb"));
        assert_eq!(recase("z-stego-0.1.0-1.x86_64.rpm", "Z-Stego_0.1.0_x64.dmg").as_deref(), Some("Z-Stego-0.1.0-1.x86_64.rpm"));
        assert_eq!(recase("zpad_1.0.0_amd64.deb", "Z-Stego_0.1.0_x64.dmg"), None);
        assert_eq!(recase("Z-Stego_0.1.0_amd64.deb", "Z-Stego_0.1.0_x64.dmg"), None);
    }
}
