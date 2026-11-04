use std::collections::VecDeque;
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
    #[error("the download stopped arriving")]
    Stalled,
    #[error("the connection ended before the whole file arrived")]
    Truncated,
    #[error("the server sent a different file than expected")]
    Mismatch,
}

impl Error {
    fn permanent(&self) -> bool {
        matches!(self, Error::Status(s) if *s < 500 && *s != 408 && *s != 429) || matches!(self, Error::Mismatch)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub done: u64,
    pub total: Option<u64>,
    pub speed: u64,
    pub source: usize,
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
    pub mirrors: &'a [String],
}

#[derive(Debug, Clone, Copy)]
pub struct Patience {
    pub stall: Duration,
    pub judge_after: Duration,
    pub slow_below: u64,
    pub worth_switching: u64,
    pub retries: u32,
}

impl Default for Patience {
    fn default() -> Self {
        Self {
            stall: Duration::from_secs(20),
            judge_after: Duration::from_secs(10),
            slow_below: 128 * 1024,
            worth_switching: 2 * 1024 * 1024,
            retries: 2,
        }
    }
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
    on_progress: impl FnMut(Progress) + Send,
) -> Result<PathBuf, Error> {
    download_with(client, req, stop, Patience::default(), on_progress).await
}

pub async fn download_with(
    client: &reqwest::Client,
    req: Request<'_>,
    stop: &Stop,
    patience: Patience,
    mut on_progress: impl FnMut(Progress) + Send,
) -> Result<PathBuf, Error> {
    if let Some(dir) = req.dest.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    if let (Ok(meta), Some(expected)) = (tokio::fs::metadata(req.dest).await, req.sha256) {
        if meta.is_file() && sha256_file(req.dest).await?.eq_ignore_ascii_case(expected) {
            on_progress(Progress { done: meta.len(), total: Some(meta.len()), speed: 0, source: 0 });
            return Ok(req.dest.to_path_buf());
        }
    }

    let mirrors = if req.sha256.is_some() { req.mirrors } else { &[] };
    let sources: Vec<&str> = std::iter::once(req.url).chain(mirrors.iter().map(String::as_str)).collect();
    let mut tried = vec![Tried::default(); sources.len()];
    let part = part_path(req.dest);
    let mut total = None;
    let mut error = None;
    let mut at = 0;
    let done = loop {
        let attempt = Attempt {
            client,
            url: sources[at],
            part: &part,
            source: at,
            stop,
            patience,
            may_switch: tried.iter().enumerate().any(|(i, t)| i != at && !t.dead && !t.slow),
        };
        match attempt.run(&mut total, &mut on_progress).await {
            Ok(Fetched::Complete(done)) => break done,
            Ok(Fetched::Slow) => tried[at].slow = true,
            Err(e @ (Error::Cancelled | Error::Io(_))) => return Err(e),
            Err(e) => {
                tried[at].failures += 1;
                tried[at].dead |= e.permanent() || tried[at].failures > patience.retries;
                if at == 0 || error.is_none() {
                    error = Some(e);
                }
            }
        }
        let Some(next) = pick(&tried) else {
            return Err(error.unwrap_or(Error::Stalled));
        };
        if next == at && tried[at].failures > 0 {
            tokio::time::sleep(patience.stall / 10 * tried[at].failures).await;
            if stop.is_stopped() {
                return Err(Error::Cancelled);
            }
        }
        at = next;
    };
    on_progress(Progress { done, total: total.or(Some(done)), speed: 0, source: at });

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

#[derive(Debug, Clone, Copy, Default)]
struct Tried {
    failures: u32,
    slow: bool,
    dead: bool,
}

fn pick(tried: &[Tried]) -> Option<usize> {
    tried
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.dead)
        .min_by_key(|(i, t)| (t.slow, t.failures, *i))
        .map(|(i, _)| i)
}

enum Fetched {
    Complete(u64),
    Slow,
}

struct Attempt<'a> {
    client: &'a reqwest::Client,
    url: &'a str,
    part: &'a Path,
    source: usize,
    stop: &'a Stop,
    patience: Patience,
    may_switch: bool,
}

impl Attempt<'_> {
    async fn run(&self, total: &mut Option<u64>, on_progress: &mut (impl FnMut(Progress) + Send)) -> Result<Fetched, Error> {
        let p = self.patience;
        let have = tokio::fs::metadata(self.part).await.map(|m| m.len()).unwrap_or(0);
        let mut get = self.client.get(self.url);
        if have > 0 {
            get = get.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let resp = tokio::time::timeout(p.stall, get.send()).await.map_err(|_| Error::Stalled)??;
        let (resume, size) = match resp.status().as_u16() {
            206 => {
                let range = content_range(&resp);
                if range.is_some_and(|(start, _)| start != have) {
                    return Err(Error::Mismatch);
                }
                (true, range.and_then(|(_, size)| size).or(resp.content_length().map(|n| n + have)))
            }
            416 if have > 0 => return Ok(Fetched::Complete(have)),
            200 => (false, resp.content_length()),
            s => return Err(Error::Status(s)),
        };
        if total.zip(size).is_some_and(|(known, size)| known != size) {
            return Err(Error::Mismatch);
        }
        *total = total.or(size);
        let (mut file, mut done) = if resume {
            (tokio::fs::OpenOptions::new().append(true).open(self.part).await?, have)
        } else {
            (tokio::fs::File::create(self.part).await?, 0)
        };
        on_progress(Progress { done, total: *total, speed: 0, source: self.source });

        let tick = Duration::from_secs(1).min(p.stall / 4).min(p.judge_after / 4).max(Duration::from_millis(5));
        let started = Instant::now();
        let mut last_data = started;
        let mut last_emit = started;
        let mut samples = VecDeque::from([(started, done)]);
        let mut speed = 0;
        let mut stream = resp.bytes_stream();
        loop {
            if self.stop.is_stopped() {
                file.flush().await?;
                return Err(Error::Cancelled);
            }
            match tokio::time::timeout(tick, stream.next()).await {
                Ok(Some(Ok(chunk))) => {
                    file.write_all(&chunk).await?;
                    done += chunk.len() as u64;
                    last_data = Instant::now();
                }
                Ok(Some(Err(e))) => {
                    file.flush().await?;
                    return Err(e.into());
                }
                Ok(None) => break,
                Err(_) => {}
            }
            let now = Instant::now();
            if samples.back().is_some_and(|(t, _)| now - *t >= tick) {
                samples.push_back((now, done));
                while samples.len() > 2 && now - samples[1].0 >= p.judge_after {
                    samples.pop_front();
                }
                speed = rate(&samples, now, done, tick * 2);
            }
            if now - last_data >= p.stall {
                file.flush().await?;
                return Err(Error::Stalled);
            }
            if self.may_switch && now - started >= p.judge_after {
                let left = total.map(|t| t.saturating_sub(done));
                if rate(&samples, now, done, p.judge_after) < p.slow_below && left.is_none_or(|l| l >= p.worth_switching) {
                    file.flush().await?;
                    return Ok(Fetched::Slow);
                }
            }
            if now - last_emit >= Duration::from_millis(100) {
                on_progress(Progress { done, total: *total, speed, source: self.source });
                last_emit = now;
            }
        }
        file.flush().await?;
        if total.is_some_and(|t| done < t) {
            return Err(Error::Truncated);
        }
        Ok(Fetched::Complete(done))
    }
}

fn content_range(resp: &reqwest::Response) -> Option<(u64, Option<u64>)> {
    let value = resp.headers().get(reqwest::header::CONTENT_RANGE)?.to_str().ok()?;
    let (range, size) = value.strip_prefix("bytes ")?.split_once('/')?;
    let start = range.split_once('-')?.0.trim().parse().ok()?;
    Some((start, size.trim().parse().ok()))
}

fn rate(samples: &VecDeque<(Instant, u64)>, now: Instant, done: u64, span: Duration) -> u64 {
    let Some(&(t, d)) = samples.iter().find(|(t, _)| now - *t <= span) else {
        return 0;
    };
    let secs = (now - t).as_secs_f64();
    if secs <= 0.0 {
        0
    } else {
        (done.saturating_sub(d) as f64 / secs) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    fn body() -> &'static [u8] {
        static BODY: OnceLock<Vec<u8>> = OnceLock::new();
        BODY.get_or_init(|| (0..64 * 1024u32).map(|i| (i * 7 % 251) as u8).collect())
    }

    #[derive(Clone, Copy)]
    enum Reply {
        Serve,
        Trickle(usize),
        StallAfter(usize),
        NotFound,
        Other(&'static [u8]),
    }

    type Plan = Arc<dyn Fn(&str, usize) -> Reply + Send + Sync>;
    type Seen = Arc<Mutex<Vec<(String, Option<usize>)>>>;

    async fn serve(plan: Plan) -> (String, Seen) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen: Seen = Arc::default();
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let (plan, log) = (plan.clone(), log.clone());
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                        let n = sock.read(&mut buf).await.unwrap();
                        if n == 0 {
                            return;
                        }
                        head.extend_from_slice(&buf[..n]);
                    }
                    let head = String::from_utf8_lossy(&head).to_lowercase();
                    let name = head.split_whitespace().nth(1).unwrap_or("/").trim_start_matches('/').split('/').next().unwrap().to_string();
                    let from = head
                        .lines()
                        .find_map(|l| l.strip_prefix("range: bytes="))
                        .and_then(|r| r.trim().trim_end_matches('-').parse::<usize>().ok());
                    let nth = {
                        let mut log = log.lock().unwrap();
                        log.push((name.clone(), from));
                        log.iter().filter(|(n, _)| *n == name).count() - 1
                    };
                    let reply = plan(&name, nth);
                    let file = match reply {
                        Reply::NotFound => {
                            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
                            return;
                        }
                        Reply::Other(other) => other,
                        _ => body(),
                    };
                    let start = from.unwrap_or(0).min(file.len());
                    let data = &file[start..];
                    let head = match from {
                        Some(_) => format!(
                            "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes {start}-{}/{}\r\nconnection: close\r\n\r\n",
                            data.len(),
                            file.len() - 1,
                            file.len()
                        ),
                        None => format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", data.len()),
                    };
                    if sock.write_all(head.as_bytes()).await.is_err() {
                        return;
                    }
                    match reply {
                        Reply::Trickle(fast) => {
                            let fast = fast.min(data.len());
                            let _ = sock.write_all(&data[..fast]).await;
                            for piece in data[fast..].chunks(64) {
                                tokio::time::sleep(Duration::from_millis(30)).await;
                                if sock.write_all(piece).await.is_err() {
                                    return;
                                }
                            }
                        }
                        Reply::StallAfter(n) => {
                            let _ = sock.write_all(&data[..n.min(data.len())]).await;
                            tokio::time::sleep(Duration::from_secs(60)).await;
                        }
                        _ => {
                            let _ = sock.write_all(data).await;
                        }
                    }
                });
            }
        });
        (base, seen)
    }

    fn quick() -> Patience {
        Patience {
            stall: Duration::from_millis(400),
            judge_after: Duration::from_millis(300),
            slow_below: 20_000,
            worth_switching: 1024,
            retries: 1,
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zl-download-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sha(b: &[u8]) -> String {
        hex::encode(Sha256::digest(b))
    }

    #[tokio::test]
    async fn downloads_resumes_and_verifies() {
        let (base, seen) = serve(Arc::new(|_, _| Reply::Serve)).await;
        let url = format!("{base}/site/file.bin");
        let dir = temp("resume");
        let dest = dir.join("file.bin");
        let client = reqwest::Client::new();
        let good = sha(body());

        std::fs::write(part_path(&dest), &body()[..10]).unwrap();
        let mut last = None;
        let p = download(&client, Request { url: &url, dest: &dest, sha256: Some(&good), mirrors: &[] }, &Stop::new(), |pr| last = Some(pr))
            .await
            .unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), body());
        assert_eq!(last.unwrap().done, body().len() as u64);
        assert_eq!(seen.lock().unwrap()[0].1, Some(10), "continued after the 10 bytes already there");

        let dest2 = dir.join("bad.bin");
        let bad = "0".repeat(64);
        let err = download(&client, Request { url: &url, dest: &dest2, sha256: Some(&bad), mirrors: &[] }, &Stop::new(), |_| {})
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Checksum { .. }));
        assert!(!dest2.exists() && !part_path(&dest2).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_crawling_source_hands_the_rest_of_the_file_to_a_mirror() {
        let (base, seen) = serve(Arc::new(|name, _| if name == "site" { Reply::Trickle(4096) } else { Reply::Serve })).await;
        let dir = temp("slow");
        let dest = dir.join("file.bin");
        let good = sha(body());
        let mirrors = [format!("{base}/github/file.bin")];
        let mut sources = Vec::new();
        let req = Request { url: &format!("{base}/site/file.bin"), dest: &dest, sha256: Some(&good), mirrors: &mirrors };
        let started = Instant::now();
        let p = download_with(&reqwest::Client::new(), req, &Stop::new(), quick(), |pr| sources.push(pr.source)).await.unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), body());
        assert!(started.elapsed() < Duration::from_secs(5), "the trickle alone would take half a minute");
        assert_eq!(sources.last(), Some(&1));
        let seen = seen.lock().unwrap().clone();
        let (name, from) = &seen[1];
        assert_eq!(name, "github");
        assert!(from.is_some_and(|f| f >= 4096), "the mirror continued the file instead of starting over: {seen:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn stalls_resume_and_dead_mirrors_fall_back() {
        let (base, seen) = serve(Arc::new(|name, nth| match (name, nth) {
            ("site", 0) => Reply::StallAfter(20_000),
            ("site", _) => Reply::Serve,
            ("tagged", _) => Reply::NotFound,
            _ => Reply::Other(b"a different build of the file"),
        }))
        .await;
        let dir = temp("stall");
        let dest = dir.join("file.bin");
        let good = sha(body());
        let mirrors = [format!("{base}/tagged/file.bin"), format!("{base}/other/file.bin")];
        let req = Request { url: &format!("{base}/site/file.bin"), dest: &dest, sha256: Some(&good), mirrors: &mirrors };
        let p = download_with(&reqwest::Client::new(), req, &Stop::new(), quick(), |_| {}).await.unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), body());
        let seen = seen.lock().unwrap().clone();
        let names: Vec<&str> = seen.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["site", "tagged", "other", "site"], "stall, missing mirror, wrong file, back home: {seen:?}");
        assert_eq!(seen[3].1, Some(20_000), "resumed where the stall left off");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    #[ignore]
    async fn live_github_continues_a_zsync_download() {
        let file = "ZCashflow_1.8.0_windows_x64-setup.exe";
        let client = reqwest::Client::builder().user_agent("zl-download test").build().unwrap();
        let sums = client.get("https://zsync.eu/zcashflow/releases/SHA256SUMS.txt").send().await.unwrap().text().await.unwrap();
        let sum = sums.lines().find(|l| l.ends_with(file)).and_then(|l| l.split_whitespace().next()).unwrap().to_string();
        let start = client
            .get(format!("https://zsync.eu/zcashflow/releases/{file}"))
            .header(reqwest::header::RANGE, "bytes=0-99999")
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(start.len(), 100_000);
        let dir = temp("live");
        let dest = dir.join(file);
        std::fs::write(part_path(&dest), &start).unwrap();
        let mirrors = [
            format!("https://github.com/TheHolyOneZ/ZCashflow/releases/download/1.8.0/{file}"),
            format!("https://github.com/TheHolyOneZ/ZCashflow/releases/download/v1.8.0/{file}"),
        ];
        let req = Request { url: "https://zsync.eu/zcashflow/releases/missing.exe", dest: &dest, sha256: Some(&sum), mirrors: &mirrors };
        let mut last = None;
        download(&client, req, &Stop::new(), |p| last = Some(p)).await.unwrap();
        assert_eq!(last.unwrap().source, 2, "the untagged guess 404s, the v-tag one has the file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn without_a_checksum_mirrors_stay_unused_and_stalls_end_with_an_error() {
        let (base, seen) = serve(Arc::new(|name, _| if name == "site" { Reply::StallAfter(100) } else { Reply::Serve })).await;
        let dir = temp("nosum");
        let dest = dir.join("file.bin");
        let mirrors = [format!("{base}/github/file.bin")];
        let req = Request { url: &format!("{base}/site/file.bin"), dest: &dest, sha256: None, mirrors: &mirrors };
        let err = download_with(&reqwest::Client::new(), req, &Stop::new(), quick(), |_| {}).await.unwrap_err();
        assert!(matches!(err, Error::Stalled), "{err}");
        let seen = seen.lock().unwrap().clone();
        assert!(seen.iter().all(|(n, _)| n == "site"), "{seen:?}");
        assert_eq!(seen.len(), 2, "one retry, then it gives up: {seen:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
