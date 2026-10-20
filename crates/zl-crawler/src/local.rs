use std::path::{Path, PathBuf};

use async_trait::async_trait;
use regex::Regex;
use url::Url;

use crate::fetch::{FetchError, Fetcher, Head, Page};

pub struct LocalFetcher {
    root: PathBuf,
    fallback: Option<Box<dyn Fetcher>>,
}

enum Answer {
    File(PathBuf),
    Listing(PathBuf),
    Redirect(Url),
    NotFound,
}

impl LocalFetcher {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), fallback: None }
    }

    pub fn with_fallback(mut self, fallback: Box<dyn Fetcher>) -> Self {
        self.fallback = Some(fallback);
        self
    }

    fn site_dir(&self, host: &str) -> Option<PathBuf> {
        let host = host.trim_start_matches("www.");
        std::fs::read_dir(&self.root).ok()?.flatten().map(|e| e.path()).find(|p| {
            p.is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case(host))
        })
    }

    fn answer(&self, url: &Url) -> Answer {
        let Some(site) = url.host_str().and_then(|h| self.site_dir(h)) else { return Answer::NotFound };
        let path = percent_encoding::percent_decode_str(url.path()).decode_utf8_lossy().to_string();
        if path.split('/').any(|s| s == ".." || s.starts_with('.')) {
            return Answer::NotFound;
        }
        let fs_path = site.join(path.trim_start_matches('/'));

        if fs_path.is_file() {
            return Answer::File(fs_path);
        }
        if fs_path.is_dir() {
            if !path.ends_with('/') {
                let mut u = url.clone();
                u.set_path(&format!("{}/", url.path()));
                return Answer::Redirect(u);
            }
            for index in ["index.html", "index.php"] {
                if fs_path.join(index).is_file() {
                    return Answer::File(fs_path.join(index));
                }
            }
            if indexes_enabled(&site, &fs_path) {
                return Answer::Listing(fs_path);
            }
        }
        match rewrite(&site, &path) {
            Some(target) => match url.join(&target) {
                Ok(u) => Answer::Redirect(u),
                Err(_) => Answer::NotFound,
            },
            None => Answer::NotFound,
        }
    }
}

#[async_trait]
impl Fetcher for LocalFetcher {
    async fn get(&self, url: &Url) -> Result<Page, FetchError> {
        let mut current = url.clone();
        for _ in 0..8 {
            let page = |status: u16, ct: &str, body: String, u: &Url| Page {
                url: u.clone(),
                status,
                content_type: Some(ct.to_string()),
                body,
            };
            match self.answer(&current) {
                Answer::File(p) => {
                    let body = std::fs::read(&p).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
                    return Ok(page(200, content_type(&p), body, &current));
                }
                Answer::Listing(dir) => return Ok(page(200, "text/html", listing(&dir), &current)),
                Answer::Redirect(next) => current = next,
                Answer::NotFound => {
                    if self.site_dir(current.host_str().unwrap_or("")).is_none() {
                        if let Some(f) = &self.fallback {
                            return f.get(&current).await;
                        }
                    }
                    return Ok(page(404, "text/html", String::new(), &current));
                }
            }
        }
        Ok(Page { url: current, status: 508, content_type: None, body: String::new() })
    }

    async fn head(&self, url: &Url) -> Result<Head, FetchError> {
        Ok(match self.answer(url) {
            Answer::File(p) => Head { status: 200, location: None, size: std::fs::metadata(p).ok().map(|m| m.len()) },
            Answer::Listing(_) => Head { status: 200, location: None, size: None },
            Answer::Redirect(u) => Head { status: 302, location: Some(u), size: None },
            Answer::NotFound => {
                if self.site_dir(url.host_str().unwrap_or("")).is_none() {
                    if let Some(f) = &self.fallback {
                        return f.head(url).await;
                    }
                }
                Head { status: 404, location: None, size: None }
            }
        })
    }
}

fn content_type(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" | "php" => "text/html",
        "json" => "application/json",
        "js" => "text/javascript",
        "txt" | "sha256" => "text/plain",
        _ => "application/octet-stream",
    }
}

fn htaccess_chain(site: &Path, dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut d = Some(dir);
    while let Some(cur) = d {
        if let Ok(text) = std::fs::read_to_string(cur.join(".htaccess")) {
            out.push((cur.to_path_buf(), text));
        }
        if cur == site {
            break;
        }
        d = cur.parent();
    }
    out
}

fn indexes_enabled(site: &Path, dir: &Path) -> bool {
    for (_, text) in htaccess_chain(site, dir) {
        for line in text.lines().map(str::trim) {
            if line.starts_with("Options") {
                if line.contains("+Indexes") {
                    return true;
                }
                if line.contains("-Indexes") {
                    return false;
                }
            }
        }
    }
    false
}

fn rewrite(site: &Path, url_path: &str) -> Option<String> {
    let rel_fs = site.join(url_path.trim_start_matches('/'));
    let mut dir = rel_fs.as_path();
    while !dir.is_dir() {
        dir = dir.parent()?;
    }
    let rule_re = Regex::new(r#"^RewriteRule\s+(\S+)\s+(\S+)(?:\s+\[([^\]]*)\])?"#).unwrap();
    let env_re = Regex::new(r#"E=([A-Za-z_][A-Za-z0-9_]*):([^,\]]+)"#).unwrap();

    for (ht_dir, text) in htaccess_chain(site, dir) {
        if !text.contains("RewriteRule") {
            continue;
        }
        let dir_url = format!(
            "/{}",
            ht_dir.strip_prefix(site).ok()?.to_string_lossy().replace('\\', "/").trim_matches('/')
        );
        let dir_url = if dir_url == "/" { dir_url } else { format!("{dir_url}/") };
        let rel = url_path.strip_prefix(dir_url.as_str()).unwrap_or(url_path.trim_start_matches('/'));
        let mut base = dir_url.clone();
        let mut env: Vec<(String, String)> = Vec::new();

        for line in text.lines().map(str::trim) {
            if let Some(b) = line.strip_prefix("RewriteBase") {
                base = b.trim().to_string();
                if !base.ends_with('/') {
                    base.push('/');
                }
                continue;
            }
            let Some(c) = rule_re.captures(line) else { continue };
            let (pat, target, flags) = (&c[1], &c[2], c.get(3).map(|m| m.as_str()).unwrap_or(""));
            if let Some(e) = env_re.captures(flags) {
                env.push((e[1].to_string(), e[2].trim().to_string()));
            }
            if pat == "^" || target == "-" || !flags.contains("R") {
                continue;
            }
            let Ok(re) = Regex::new(pat) else { continue };
            let Some(m) = re.captures(rel) else { continue };
            let mut t = target.to_string();
            for (k, v) in &env {
                t = t.replace(&format!("%{{ENV:{k}}}"), v);
            }
            t = t.replace("%1", &dir_url);
            for i in 1..m.len() {
                t = t.replace(&format!("${i}"), m.get(i).map(|x| x.as_str()).unwrap_or(""));
            }
            if t.contains("%{") {
                continue;
            }
            if !(t.starts_with('/') || t.contains("://")) {
                t = format!("{base}{t}");
            }
            return Some(t);
        }
        return None;
    }
    None
}

fn listing(dir: &Path) -> String {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    if n.starts_with('.') {
                        return None;
                    }
                    Some(if e.path().is_dir() { format!("{n}/") } else { n })
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    let rows: String = names
        .iter()
        .map(|n| {
            let href = percent_encoding::utf8_percent_encode(n, percent_encoding::NON_ALPHANUMERIC)
                .to_string()
                .replace("%2E", ".")
                .replace("%2F", "/")
                .replace("%2D", "-")
                .replace("%5F", "_");
            format!("<tr><td><a href=\"{href}\">{n}</a></td></tr>\n")
        })
        .collect();
    format!("<html><head><title>Index of</title></head><body><h1>Index of</h1><table>\n<tr><td><a href=\"../\">Parent Directory</a></td></tr>\n{rows}</table></body></html>")
}
