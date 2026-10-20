use serde::Deserialize;
use url::Url;

use crate::fetch::Fetcher;
use crate::manifest::Entry;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    #[allow(dead_code)]
    name: String,
    browser_download_url: String,
    size: Option<u64>,
    digest: Option<String>,
}

#[derive(Debug, Default)]
pub struct Latest {
    pub entries: Vec<Entry>,
    pub notes: Option<String>,
    pub date: Option<String>,
}

fn latest_of(r: Release) -> Latest {
    let notes = r.body.clone().map(|b| b.trim().to_string()).filter(|b| !b.is_empty());
    let date = r.published_at.clone();
    Latest { entries: entries(r), notes, date }
}

fn entries(r: Release) -> Vec<Entry> {
    let version = Some(r.tag_name.trim_start_matches('v').to_string());
    r.assets
        .into_iter()
        .filter_map(|a| {
            Some(Entry {
                url: Url::parse(&a.browser_download_url).ok()?,
                version: version.clone(),
                size: a.size,
                sha256: a.digest.and_then(|d| d.strip_prefix("sha256:").map(str::to_string)),
            })
        })
        .collect()
}

pub fn parse_releases(body: &str) -> Latest {
    if let Ok(r) = serde_json::from_str::<Release>(body) {
        return latest_of(r);
    }
    serde_json::from_str::<Vec<Release>>(body)
        .ok()
        .and_then(|list| list.into_iter().find(|r| !r.draft && !r.assets.is_empty()))
        .map(latest_of)
        .unwrap_or_default()
}

pub async fn latest(fetcher: &dyn Fetcher, api_base: &Url, owner: &str, repo: &str) -> Latest {
    let Ok(latest) = api_base.join(&format!("repos/{owner}/{repo}/releases/latest")) else { return Latest::default() };
    if let Ok(page) = fetcher.get(&latest).await {
        if page.ok() {
            let l = parse_releases(&page.body);
            if !l.entries.is_empty() {
                return l;
            }
        }
        if page.status == 403 || page.status == 429 {
            tracing::warn!("GitHub rate limit reached while asking for {owner}/{repo}");
            return Latest::default();
        }
    }
    let Ok(list) = api_base.join(&format!("repos/{owner}/{repo}/releases?per_page=5")) else { return Latest::default() };
    match fetcher.get(&list).await {
        Ok(page) if page.ok() => parse_releases(&page.body),
        _ => Latest::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_assets() {
        let body = r#"{"tag_name":"v1.4.0","draft":false,"assets":[{"name":"ZKeyDebouncer_1.4.0_x64-setup.exe",
            "browser_download_url":"https://github.com/TheHolyOneZ/ZKeyDebouncer/releases/download/v1.4.0/ZKeyDebouncer_1.4.0_x64-setup.exe",
            "size":123,"digest":"sha256:abc"}]}"#;
        let l = parse_releases(body);
        assert_eq!(l.entries[0].version.as_deref(), Some("1.4.0"));
        assert_eq!(l.entries[0].sha256.as_deref(), Some("abc"));
    }
}
