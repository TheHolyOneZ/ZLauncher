use std::path::PathBuf;
use std::sync::Arc;

use zl_catalog::{Link, Project};
use zl_crawler::{Crawler, Kind, LocalFetcher, Os, Source};

fn websites() -> Option<PathBuf> {
    let dir = std::env::var_os("ZL_WEBSITES")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Documents/Websites")))?;
    dir.join("ZSync.eu").is_dir().then_some(dir)
}

fn project(id: &str, url: &str) -> Project {
    Project {
        id: id.into(),
        title: id.into(),
        category: None,
        description: String::new(),
        url: Some(url.into()),
        links: vec![Link { label: "Download".into(), url: url.into(), primary: true }],
        categories: vec![],
        added: None,
        retired: false,
        video: None,
    }
}

#[tokio::test]
async fn known_cases() {
    let Some(dir) = websites() else {
        eprintln!("local websites not found, skipping");
        return;
    };
    let mut crawler = Crawler::new(Arc::new(LocalFetcher::new(dir)));
    crawler.use_github = false;

    let cases = [
        ("zfontmanager", "https://zsync.eu/zfontmanager/", Kind::Dmg, Source::Link),
        ("zhell", "https://zsync.eu/zhell/", Kind::Deb, Source::Alias),
        ("ztally", "https://zsync.eu/ztally/", Kind::AppImage, Source::Alias),
        ("zdraft", "https://zsync.eu/zdraft/", Kind::AppImage, Source::Checksums),
        ("zlynstall", "https://zsync.eu/zlynstall/", Kind::Rpm, Source::Manifest),
        ("zgamelib", "https://zsync.eu/zgamelib/", Kind::Msi, Source::Manifest),
        ("zdbf", "https://zsync.eu/zdbf/", Kind::Setup, Source::Link),
        ("zwebwrap", "https://zsync.eu/zwebwrap/", Kind::Deb, Source::Link),
        ("zit-suite", "https://zsync.eu/zit-suite/", Kind::Deb, Source::Link),
        ("zyringe", "https://github.com/TheHolyOneZ/Zyringe", Kind::AppImage, Source::Link),
    ];
    for (id, url, kind, source) in cases {
        let r = crawler.crawl(&project(id, url)).await;
        let hit = r.artifacts.iter().find(|a| a.kind == kind);
        let hit = hit.unwrap_or_else(|| panic!("{id}: no {kind:?} in {:#?}", r));
        assert_eq!(hit.source, source, "{id}");
        assert!(hit.size.is_some(), "{id}: size");
    }

    let r = crawler.crawl(&project("zhell", "https://zsync.eu/zhell/")).await;
    assert_eq!(r.version.as_deref(), Some("0.1.1"));
    assert!(r.artifacts.iter().any(|a| a.os == Os::Windows));

    let r = crawler.crawl(&project("modmenus", "https://zlogic.eu/mods/")).await;
    assert!(r.artifacts.is_empty());
}
