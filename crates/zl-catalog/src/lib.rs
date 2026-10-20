use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const DEFAULT_BASE: &str = "https://zsync.eu/";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Link {
    pub label: String,
    pub url: String,
    #[serde(default)]
    pub primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Video {
    pub mp4: String,
    pub poster: Option<String>,
    pub duration: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub id: String,
    pub title: String,
    pub category: Option<String>,
    pub description: String,
    pub url: Option<String>,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub categories: Vec<String>,
    pub added: Option<String>,
    #[serde(default)]
    pub retired: bool,
    #[serde(default)]
    pub video: Option<Video>,
}

impl Project {
    pub fn github_repo(&self) -> Option<(String, String)> {
        self.links.iter().find_map(|l| parse_github(&l.url))
    }
}

pub fn parse_github(link: &str) -> Option<(String, String)> {
    let u = url::Url::parse(link).ok()?;
    if u.host_str()? != "github.com" {
        return None;
    }
    let mut seg = u.path_segments()?.filter(|s| !s.is_empty());
    let owner = seg.next()?.to_string();
    let repo = seg.next()?.trim_end_matches(".git").to_string();
    Some((owner, repo))
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the project list is unavailable right now (retry in {retry_after:?})")]
    Unavailable { retry_after: Option<Duration> },
    #[error("network error: {0}")]
    Http(String),
    #[error("unexpected answer from {url}: {status}")]
    Status { url: String, status: u16 },
    #[error("bad catalogue data: {0}")]
    Parse(#[from] serde_json::Error),
}

#[derive(Deserialize)]
struct ProjectsResponse {
    projects: Vec<RawProject>,
}

#[derive(Deserialize)]
struct RawProject {
    #[serde(default)]
    id: String,
    title: String,
    category: Option<String>,
    description: String,
    url: Option<String>,
    #[serde(default)]
    links: Vec<Link>,
    #[serde(default)]
    categories: Vec<String>,
    added: Option<String>,
    #[serde(default)]
    retired: bool,
}

#[derive(Deserialize)]
struct RawVideo {
    mp4: String,
    poster: Option<String>,
    duration: Option<u32>,
}

pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

pub fn parse_projects(body: &str) -> Result<Vec<Project>, Error> {
    let resp: ProjectsResponse = serde_json::from_str(body)?;
    Ok(resp
        .projects
        .into_iter()
        .map(|p| Project {
            id: if p.id.trim().is_empty() { slug(&p.title) } else { p.id },
            title: p.title,
            category: p.category,
            description: p.description,
            url: p.url,
            links: p.links,
            categories: p.categories,
            added: p.added,
            retired: p.retired,
            video: None,
        })
        .collect())
}

pub fn parse_videos(body: &str, base: &url::Url) -> Result<HashMap<String, Video>, Error> {
    let raw: HashMap<String, RawVideo> = serde_json::from_str(body)?;
    let abs = |p: &str| base.join(p).map(|u| u.to_string()).unwrap_or_else(|_| p.to_string());
    Ok(raw
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                Video { mp4: abs(&v.mp4), poster: v.poster.as_deref().map(abs), duration: v.duration },
            )
        })
        .collect())
}

pub fn join_videos(projects: &mut [Project], videos: &HashMap<String, Video>) {
    for p in projects {
        p.video = videos.get(&p.id).cloned();
    }
}

#[cfg(feature = "http")]
impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Http(e.to_string())
    }
}

#[cfg(feature = "http")]
pub async fn fetch(client: &reqwest::Client, base: &str) -> Result<Vec<Project>, Error> {
    let base = url::Url::parse(base).expect("catalogue base must be a valid URL");
    let projects_url = base.join("projects.php").expect("static path");
    let resp = client.get(projects_url.clone()).send().await?;
    let status = resp.status();
    if status.as_u16() == 503 {
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        return Err(Error::Unavailable { retry_after });
    }
    if !status.is_success() {
        return Err(Error::Status { url: projects_url.to_string(), status: status.as_u16() });
    }
    let mut projects = parse_projects(&resp.text().await?)?;
    if projects.is_empty() {
        return Err(Error::Unavailable { retry_after: None });
    }

    let videos_url = base.join("motion-graphics/videos.json").expect("static path");
    if let Ok(resp) = client.get(videos_url).send().await {
        if resp.status().is_success() {
            if let Ok(body) = resp.text().await {
                if let Ok(videos) = parse_videos(&body, &base) {
                    join_videos(&mut projects, &videos);
                }
            }
        }
    }
    Ok(projects)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_projects_and_joins_videos() {
        let body = r#"{"projects":[{"title":"ZCashflow","category":"Writing & productivity",
            "description":"d","url":"https://zsync.eu/zcashflow/","links":[
            {"label":"Download","url":"https://zsync.eu/zcashflow/","primary":true},
            {"label":"Source","url":"https://github.com/TheHolyOneZ/ZCashflow","primary":false}],
            "id":"zcashflow","categories":[],"added":"2026-01-01","retired":false},
            {"title":"Grammar Fixer AI","description":"x","url":null,"links":[],"id":""}],"count":2}"#;
        let mut p = parse_projects(body).unwrap();
        assert_eq!(p[1].id, "grammar-fixer-ai");
        assert_eq!(p[0].github_repo(), Some(("TheHolyOneZ".into(), "ZCashflow".into())));

        let base = url::Url::parse("https://zsync.eu/").unwrap();
        let v = parse_videos(
            r#"{"zcashflow":{"mp4":"motion-graphics/zcashflow.mp4","poster":"motion-graphics/zcashflow.jpg","duration":45}}"#,
            &base,
        )
        .unwrap();
        join_videos(&mut p, &v);
        assert_eq!(p[0].video.as_ref().unwrap().mp4, "https://zsync.eu/motion-graphics/zcashflow.mp4");
        assert!(p[1].video.is_none());
    }
}
