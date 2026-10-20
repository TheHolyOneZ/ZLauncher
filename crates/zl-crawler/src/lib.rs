pub mod classify;
pub mod crawl;
pub mod fetch;
pub mod github;
pub mod local;
pub mod manifest;
pub mod model;
pub mod overrides;
pub mod scan;

pub use classify::{classify, cmp_versions};
pub use crawl::Crawler;
pub use fetch::Fetcher;
#[cfg(feature = "http")]
pub use fetch::HttpFetcher;
pub use local::LocalFetcher;
pub use model::{Arch, Artifact, CrawlReport, Kind, Os, Source};
