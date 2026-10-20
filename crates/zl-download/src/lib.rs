use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("the server answered {0}")]
    Status(u16),
    #[error("disk error: {0}")]
    Io(#[from] std::io::Error),
    #[error("stopped")]
    Cancelled,
    #[error("checksum mismatch: expected {expected}, got {actual}. The file was deleted.")]
    Checksum { expected: String, actual: String },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub done: u64,
    pub total: Option<u64>,
    pub speed: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Stop(Arc<AtomicBool>);

impl Stop {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn stop(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_stopped(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

pub struct Request<'a> {
    pub url: &'a str,
    pub dest: &'a Path,
    pub sha256: Option<&'a str>,
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    dest.with_file_name(name)
}

pub async fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub async fn download(
    client: &reqwest::Client,
    req: Request<'_>,
    stop: &Stop,
    mut on_progress: impl FnMut(Progress) + Send,
) -> Result<PathBuf, Error> {
    if let Some(dir) = req.dest.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    if let (Ok(meta), Some(expected)) = (tokio::fs::metadata(req.dest).await, req.sha256) {
        if meta.is_file() && sha256_file(req.dest).await?.eq_ignore_ascii_case(expected) {
            on_progress(Progress { done: meta.len(), total: Some(meta.len()), speed: 0 });
            return Ok(req.dest.to_path_buf());
        }
    }

    let part = part_path(req.dest);
    let have = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
    let mut get = client.get(req.url);
    if have > 0 {
        get = get.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let resp = get.send().await?;
    let status = resp.status().as_u16();

    let (mut file, mut done, total) = match status {
        206 => {
            let f = tokio::fs::OpenOptions::new().append(true).open(&part).await?;
            (f, have, resp.content_length().map(|n| n + have))
        }
        416 if have > 0 => {
            let f = tokio::fs::OpenOptions::new().append(true).open(&part).await?;
            (f, have, Some(have))
        }
        200 => (tokio::fs::File::create(&part).await?, 0, resp.content_length()),
        s => return Err(Error::Status(s)),
    };

    if status != 416 {
        let mut stream = resp.bytes_stream();
        let (mut last, mut window_start, mut window_bytes, mut speed) = (Instant::now(), Instant::now(), 0u64, 0u64);
        on_progress(Progress { done, total, speed });
        while let Some(chunk) = stream.next().await {
            if stop.is_stopped() {
                file.flush().await?;
                return Err(Error::Cancelled);
            }
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            done += chunk.len() as u64;
            window_bytes += chunk.len() as u64;
            let elapsed = window_start.elapsed();
            if elapsed >= Duration::from_secs(1) {
                speed = (window_bytes as f64 / elapsed.as_secs_f64()) as u64;
                window_start = Instant::now();
                window_bytes = 0;
            }
            if last.elapsed() >= Duration::from_millis(100) {
                on_progress(Progress { done, total, speed });
                last = Instant::now();
            }
        }
        file.flush().await?;
    }
    drop(file);
    on_progress(Progress { done, total: total.or(Some(done)), speed: 0 });

    if let Some(expected) = req.sha256 {
        let actual = sha256_file(&part).await?;
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(Error::Checksum { expected: expected.to_ascii_lowercase(), actual });
        }
    }
    tokio::fs::rename(&part, req.dest).await?;
    Ok(req.dest.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    async fn serve(body: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = sock.read(&mut buf).await.unwrap();
                    let req = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                    let from = req
                        .lines()
                        .find_map(|l| l.strip_prefix("range: bytes="))
                        .and_then(|r| r.trim_end_matches('-').trim().trim_end_matches('-').parse::<usize>().ok());
                    let (status, data) = match from {
                        Some(f) => ("206 Partial Content", &body[f..]),
                        None => ("200 OK", body),
                    };
                    let head = format!("HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", data.len());
                    sock.write_all(head.as_bytes()).await.unwrap();
                    sock.write_all(data).await.unwrap();
                });
            }
        });
        format!("http://{addr}/file.deb")
    }

    #[tokio::test]
    async fn downloads_resumes_and_verifies() {
        const BODY: &[u8] = b"hello zlauncher, this is a release file";
        let url = serve(BODY).await;
        let dir = std::env::temp_dir().join(format!("zl-download-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dest = dir.join("file.deb");
        let client = reqwest::Client::new();
        let good = hex::encode(Sha256::digest(BODY));

        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(part_path(&dest), &BODY[..10]).unwrap();
        let mut last = None;
        let p = download(&client, Request { url: &url, dest: &dest, sha256: Some(&good) }, &Stop::new(), |pr| {
            last = Some(pr)
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), BODY);
        assert_eq!(last.unwrap().done, BODY.len() as u64);

        let dest2 = dir.join("bad.deb");
        let err = download(&client, Request { url: &url, dest: &dest2, sha256: Some(&"0".repeat(64)) }, &Stop::new(), |_| {})
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Checksum { .. }));
        assert!(!dest2.exists() && !part_path(&dest2).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
