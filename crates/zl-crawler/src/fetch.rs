use async_trait::async_trait;
use url::Url;

#[cfg(feature = "http")]
use std::time::Duration;

#[cfg(feature = "http")]
const MAX_TEXT: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Page {
    pub url: Url,
    pub status: u16,
    pub content_type: Option<String>,
    pub body: String,
}

impl Page {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

#[derive(Debug, Clone)]
pub struct Head {
    pub status: u16,
    pub location: Option<Url>,
    pub size: Option<u64>,
}

impl Head {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("{0}")]
    Http(String),
    #[error("{0} is too big to scan")]
    TooBig(Url),
}

#[async_trait]
pub trait Fetcher: Send + Sync {
    async fn get(&self, url: &Url) -> Result<Page, FetchError>;
    async fn head(&self, url: &Url) -> Result<Head, FetchError>;
}

#[cfg(feature = "http")]
impl From<reqwest::Error> for FetchError {
    fn from(e: reqwest::Error) -> Self {
        FetchError::Http(e.to_string())
    }
}

pub type GithubToken = std::sync::Arc<std::sync::RwLock<Option<String>>>;

#[cfg(feature = "http")]
pub struct HttpFetcher {
    follow: reqwest::Client,
    no_follow: reqwest::Client,
    github_token: GithubToken,
}

pub fn user_agent() -> String {
    format!("ZLauncher/{} (+https://zsync.eu/)", env!("CARGO_PKG_VERSION"))
}

#[cfg(feature = "http")]
impl HttpFetcher {
    pub fn new() -> Self {
        let build = |policy| {
            reqwest::Client::builder()
                .user_agent(user_agent())
                .timeout(Duration::from_secs(20))
                .connect_timeout(Duration::from_secs(10))
                .redirect(policy)
                .build()
                .expect("HTTP client")
        };
        Self {
            follow: build(reqwest::redirect::Policy::limited(8)),
            no_follow: build(reqwest::redirect::Policy::none()),
            github_token: GithubToken::default(),
        }
    }

    pub fn with_github_token(mut self, token: GithubToken) -> Self {
        self.github_token = token;
        self
    }
}

#[cfg(feature = "http")]
impl Default for HttpFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "http")]
#[async_trait]
impl Fetcher for HttpFetcher {
    async fn get(&self, url: &Url) -> Result<Page, FetchError> {
        let mut req = self.follow.get(url.clone());
        if url.host_str() == Some("api.github.com") {
            req = req.header("Accept", "application/vnd.github+json");
            if let Some(t) = self.github_token.read().ok().and_then(|t| t.clone()) {
                req = req.bearer_auth(t);
            }
        }
        let resp = req.send().await?;
        if resp.content_length().is_some_and(|n| n > MAX_TEXT) {
            return Err(FetchError::TooBig(url.clone()));
        }
        let status = resp.status().as_u16();
        let final_url = resp.url().clone();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = if content_type.as_deref().is_some_and(|t| t.starts_with("application/octet-stream")) {
            String::new()
        } else {
            resp.text().await?
        };
        Ok(Page { url: final_url, status, content_type, body })
    }

    async fn head(&self, url: &Url) -> Result<Head, FetchError> {
        let resp = self.no_follow.head(url.clone()).send().await?;
        let status = resp.status().as_u16();
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|l| url.join(l).ok());
        let size = resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok());
        Ok(Head { status, location, size })
    }
}

pub async fn resolve(fetcher: &dyn Fetcher, url: &Url, max: usize) -> Result<(Url, Head), FetchError> {
    let mut current = url.clone();
    for _ in 0..max {
        let head = fetcher.head(&current).await?;
        match (&head.location, (300..400).contains(&head.status)) {
            (Some(next), true) => {
                let mut next = next.clone();
                if next.scheme() == "http" && current.scheme() == "https" {
                    let _ = next.set_scheme("https");
                }
                current = next;
            }
            _ => return Ok((current, head)),
        }
    }
    let head = fetcher.head(&current).await?;
    Ok((current, head))
}
