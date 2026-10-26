use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use zl_catalog::Project;
use zl_crawler::{cmp_versions, Artifact, CrawlReport, Crawler, HttpFetcher, Os};
use zl_install::{installable, recommend, Host, LinuxFormat};
use zl_store::{now, Install, Store};

const SETTINGS_KEY: &str = "settings";
const CATALOGUE_TTL: i64 = 10 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub crawl_hours: u32,
    pub linux_format: LinuxFormat,
    pub desktop_shortcut: bool,
    pub theme: String,
    pub delete_after_install: bool,
    pub hover_video: bool,
    pub favorites: Vec<String>,
    pub sort: String,
    pub notify_updates: bool,
    pub system_frame: bool,
    pub ask_admin_on_start: bool,
    pub auto_update: bool,
    pub assistant_enabled: bool,
    pub assistant_provider: String,
    pub assistant_model: String,
    pub assistant_base_url: String,
    pub assistant_thinking: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            crawl_hours: 6,
            linux_format: LinuxFormat::Auto,
            desktop_shortcut: false,
            theme: "system".into(),
            delete_after_install: true,
            hover_video: true,
            favorites: Vec::new(),
            sort: "newest".into(),
            notify_updates: true,
            system_frame: false,
            ask_admin_on_start: true,
            auto_update: true,
            assistant_enabled: true,
            assistant_provider: "ollama".into(),
            assistant_model: "qwen2.5".into(),
            assistant_base_url: "http://localhost:11434".into(),
            assistant_thinking: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    #[serde(flatten)]
    pub project: Project,
    pub artifacts: Vec<Artifact>,
    pub version: Option<String>,
    pub release_notes: Option<String>,
    pub released: Option<String>,
    pub crawled_at: Option<i64>,
    pub recommended: Option<String>,
    pub installed: Option<InstalledView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledView {
    pub version: Option<String>,
    pub method: String,
    pub filename: String,
    pub installed_at: i64,
    pub present: bool,
    pub desktop_shortcut: bool,
    pub update: Option<String>,
    pub external: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub host: Host,
    pub projects: Vec<ProjectView>,
    pub fetched_at: Option<i64>,
    pub crawling: Option<(usize, usize)>,
    pub catalogue_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Queued,
    Building,
    Downloading,
    Installing,
    Removing,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub project_id: String,
    pub operation: String,
    pub title: String,
    pub filename: String,
    pub phase: Phase,
    pub done: u64,
    pub total: Option<u64>,
    pub speed: u64,
    pub step: Option<String>,
    pub log: Vec<String>,
    pub error: Option<String>,
    pub hint: Option<String>,
    pub saved_to: Option<String>,
    pub needs_auth: bool,
    pub missing: Vec<zl_build::Missing>,
    pub built_from: Option<String>,
}

pub(crate) struct Job {
    pub(crate) view: JobView,
    pub(crate) stop: zl_download::Stop,
    pub(crate) cancel: Arc<AtomicBool>,
}

impl Job {
    pub(crate) fn new(
        project_id: &str,
        operation: &str,
        title: &str,
        filename: &str,
        phase: Phase,
        total: Option<u64>,
    ) -> Self {
        Job {
            view: JobView {
                project_id: project_id.to_string(),
                operation: operation.to_string(),
                title: title.to_string(),
                filename: filename.to_string(),
                phase,
                done: 0,
                total,
                speed: 0,
                step: None,
                log: Vec::new(),
                error: None,
                hint: None,
                saved_to: None,
                needs_auth: false,
                missing: Vec::new(),
                built_from: None,
            },
            stop: zl_download::Stop::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub(crate) fn busy(p: &Phase) -> bool {
    matches!(
        p,
        Phase::Queued | Phase::Building | Phase::Downloading | Phase::Installing | Phase::Removing
    )
}

pub struct App {
    pub store: Arc<Store>,
    settings: std::sync::RwLock<Settings>,
    settings_writer: std::sync::mpsc::Sender<Settings>,
    pub host: Host,
    pub(crate) client: reqwest::Client,
    crawler: Crawler,
    pub(crate) github_token: zl_crawler::fetch::GithubToken,
    downloads: PathBuf,
    pub(crate) builds: PathBuf,
    pub(crate) jobs: Mutex<HashMap<String, Job>>,
    pub(crate) assistant_chats_lock: Mutex<()>,
    pub(crate) external: Mutex<HashMap<String, zl_install::detect::External>>,
    crawling: Mutex<Option<(usize, usize)>>,
    catalogue_error: Mutex<Option<String>>,
    refreshing: AtomicBool,
}

pub type Shared = Arc<App>;

fn spawn_settings_writer(store: Arc<Store>) -> std::sync::mpsc::Sender<Settings> {
    let (tx, rx) = std::sync::mpsc::channel::<Settings>();
    std::thread::Builder::new()
        .name("settings-writer".into())
        .spawn(move || {
            while let Ok(mut s) = rx.recv() {
                std::thread::sleep(Duration::from_millis(150));
                while let Ok(newer) = rx.try_recv() {
                    s = newer;
                }
                if let Err(e) = store.set_setting(SETTINGS_KEY, &s) {
                    tracing::warn!("could not save settings: {e}");
                }
            }
        })
        .expect("settings writer thread");
    tx
}

impl App {
    pub fn new(data_dir: PathBuf, cache_dir: PathBuf) -> anyhow::Result<Self> {
        let store = Arc::new(Store::open(&data_dir.join("zlauncher.db"))?);
        let settings: Settings = store
            .setting(SETTINGS_KEY)
            .ok()
            .flatten()
            .unwrap_or_default();
        let settings_writer = spawn_settings_writer(store.clone());
        let client = reqwest::Client::builder()
            .user_agent(zl_crawler::fetch::user_agent())
            .connect_timeout(Duration::from_secs(15))
            .build()?;
        let github_token: zl_crawler::fetch::GithubToken = Arc::new(std::sync::RwLock::new(
            store
                .setting::<String>(crate::github::TOKEN_KEY)
                .ok()
                .flatten(),
        ));
        Ok(Self {
            store,
            settings: std::sync::RwLock::new(settings),
            settings_writer,
            host: zl_install::host(),
            client,
            crawler: Crawler::new(Arc::new(
                HttpFetcher::new().with_github_token(github_token.clone()),
            )),
            github_token,
            downloads: cache_dir.join("downloads"),
            builds: cache_dir.join("builds"),
            jobs: Mutex::new(HashMap::new()),
            assistant_chats_lock: Mutex::new(()),
            external: Mutex::new(HashMap::new()),
            crawling: Mutex::new(None),
            catalogue_error: Mutex::new(None),
            refreshing: AtomicBool::new(false),
        })
    }

    pub fn settings(&self) -> Settings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn flush_settings(&self) {
        if let Err(e) = self.store.set_setting(SETTINGS_KEY, &self.settings()) {
            tracing::warn!("could not save settings: {e}");
        }
    }

    pub fn set_settings(&self, s: &Settings) -> Result<(), String> {
        *self.settings.write().unwrap_or_else(|e| e.into_inner()) = s.clone();
        self.settings_writer
            .send(s.clone())
            .map_err(|_| "The settings writer stopped.".to_string())
    }

    fn projects(&self) -> Vec<Project> {
        self.store
            .catalogue()
            .ok()
            .flatten()
            .map(|(p, _)| p)
            .unwrap_or_default()
    }

    pub(crate) fn project(&self, id: &str) -> Option<Project> {
        self.projects().into_iter().find(|p| p.id == id)
    }

    pub(crate) fn report(&self, id: &str) -> Option<CrawlReport> {
        self.store.crawl(id).ok().flatten().map(|(r, _)| r)
    }

    pub fn library(&self) -> Library {
        let settings = self.settings();
        let (projects, fetched_at) = match self.store.catalogue().ok().flatten() {
            Some((p, at)) => (p, Some(at)),
            None => (Vec::new(), None),
        };
        let crawls: HashMap<String, (CrawlReport, i64)> = self
            .store
            .crawls()
            .unwrap_or_default()
            .into_iter()
            .map(|(r, at)| (r.project_id.clone(), (r, at)))
            .collect();
        let external = self.external.lock().unwrap().clone();
        let installs: HashMap<String, Install> = self
            .store
            .installs()
            .unwrap_or_default()
            .into_iter()
            .map(|i| (i.project_id.clone(), i))
            .collect();

        let projects = projects
            .into_iter()
            .map(|project| {
                let (artifacts, version, release_notes, released, crawled_at) =
                    match crawls.get(&project.id) {
                        Some((r, at)) => (
                            r.artifacts.clone(),
                            r.version.clone(),
                            r.release_notes.clone(),
                            r.released.clone(),
                            Some(*at),
                        ),
                        None => (Vec::new(), None, None, None, None),
                    };
                let rec = recommend(&artifacts, &self.host, settings.linux_format);
                let newer =
                    |have: &Option<String>| match (rec.and_then(|a| a.version.clone()), have) {
                        (Some(new), Some(have)) if cmp_versions(&new, have).is_gt() => Some(new),
                        _ => None,
                    };
                let installed = installs
                    .get(&project.id)
                    .map(|i| InstalledView {
                        version: i.version.clone(),
                        method: i.method.clone(),
                        filename: i.filename.clone(),
                        installed_at: i.installed_at,
                        present: zl_install::is_present(&i.method, &i.handle),
                        desktop_shortcut: zl_install::has_desktop_shortcut(&i.method, &i.handle),
                        update: newer(&i.version),
                        external: None,
                    })
                    .or_else(|| {
                        external.get(&project.id).map(|e| InstalledView {
                            version: e.version.clone(),
                            method: "external".into(),
                            filename: e.name.clone(),
                            installed_at: 0,
                            present: true,
                            desktop_shortcut: false,
                            update: newer(&e.version),
                            external: Some(e.via.clone()),
                        })
                    });
                ProjectView {
                    recommended: rec.map(|a| a.url.clone()),
                    project,
                    artifacts,
                    version,
                    release_notes,
                    released,
                    crawled_at,
                    installed,
                }
            })
            .collect();
        Library {
            host: self.host.clone(),
            projects,
            fetched_at,
            crawling: *self.crawling.lock().unwrap(),
            catalogue_error: self.catalogue_error.lock().unwrap().clone(),
        }
    }

    pub async fn rescan(self: &Arc<Self>) {
        let os = Os::current();
        let products: HashMap<String, Vec<String>> = self
            .store
            .crawls()
            .unwrap_or_default()
            .into_iter()
            .map(|(r, _)| {
                let mut names: Vec<String> = r
                    .artifacts
                    .iter()
                    .filter(|a| a.os == os)
                    .map(|a| zl_install::detect::product_of(&a.filename))
                    .collect();
                names.sort();
                names.dedup();
                (r.project_id, names)
            })
            .collect();
        if let Ok(found) =
            tokio::task::spawn_blocking(move || zl_install::detect::scan(&products)).await
        {
            tracing::info!(
                "{} tools found that were installed outside the launcher",
                found.len()
            );
            *self.external.lock().unwrap() = found;
        }
    }

    pub fn jobs(&self) -> Vec<JobView> {
        self.jobs
            .lock()
            .unwrap()
            .values()
            .map(|j| j.view.clone())
            .collect()
    }

    pub(crate) fn update_job(&self, app: &AppHandle, id: &str, f: impl FnOnce(&mut JobView)) {
        let view = {
            let mut jobs = self.jobs.lock().unwrap();
            let Some(job) = jobs.get_mut(id) else { return };
            f(&mut job.view);
            job.view.clone()
        };
        let _ = app.emit("job", view);
    }

    pub fn clear_job(&self, id: &str) {
        let mut jobs = self.jobs.lock().unwrap();
        if jobs
            .get(id)
            .is_some_and(|j| matches!(j.view.phase, Phase::Done | Phase::Failed | Phase::Cancelled))
        {
            jobs.remove(id);
        }
    }

    pub fn cancel(&self, id: &str) {
        if let Some(j) = self.jobs.lock().unwrap().get(id) {
            j.stop.stop();
            j.cancel.store(true, Ordering::SeqCst);
        }
    }
}

pub async fn refresh(app: AppHandle, state: Shared, force: bool) {
    if state.refreshing.swap(true, Ordering::SeqCst) {
        return;
    }
    let fresh = state
        .store
        .catalogue()
        .ok()
        .flatten()
        .is_some_and(|(_, at)| now() - at < CATALOGUE_TTL);
    if force || !fresh {
        match zl_catalog::fetch(&state.client, zl_catalog::DEFAULT_BASE).await {
            Ok(projects) => {
                let _ = state.store.save_catalogue(&projects);
                *state.catalogue_error.lock().unwrap() = None;
            }
            Err(e) => *state.catalogue_error.lock().unwrap() = Some(e.to_string()),
        }
        let _ = app.emit("library", ());
    }

    let max_age = state.settings().crawl_hours.max(1) as i64 * 3600;
    let crawled: HashMap<String, i64> = state
        .store
        .crawls()
        .unwrap_or_default()
        .into_iter()
        .map(|(r, at)| (r.project_id, at))
        .collect();
    let stale: Vec<Project> = state
        .projects()
        .into_iter()
        .filter(|p| force || crawled.get(&p.id).is_none_or(|at| now() - at > max_age))
        .collect();

    if !stale.is_empty() {
        let total = stale.len();
        *state.crawling.lock().unwrap() = Some((0, total));
        let _ = app.emit("library", ());
        let mut stream = futures::stream::iter(stale)
            .map(|p| {
                let state = state.clone();
                async move { state.crawler.crawl(&p).await }
            })
            .buffer_unordered(4)
            .boxed();
        let mut done = 0;
        while let Some(report) = stream.next().await {
            done += 1;
            let _ = state.store.save_crawl(&report);
            *state.crawling.lock().unwrap() = Some((done, total));
            let _ = app.emit("library", ());
        }
        *state.crawling.lock().unwrap() = None;
    }
    state.rescan().await;
    let _ = app.emit("library", ());
    crate::notify::updates(&app, &state);
    state.refreshing.store(false, Ordering::SeqCst);
}

fn user_downloads() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|d| d.download_dir().map(|p| p.to_path_buf()))
        .unwrap_or_else(std::env::temp_dir)
        .join("ZLauncher")
}

pub fn start_install(
    app: AppHandle,
    state: Shared,
    project_id: String,
    url: Option<String>,
) -> Result<(), String> {
    let project = state.project(&project_id).ok_or("Unknown project.")?;
    let report = state
        .report(&project_id)
        .ok_or("No downloads found for this project yet.")?;
    let settings = state.settings();
    let installed_kind = state
        .store
        .install(&project_id)
        .ok()
        .flatten()
        .and_then(|i| zl_crawler::classify(&i.filename))
        .map(|c| (c.kind, c.arch));
    let artifact = match &url {
        Some(u) => report.artifacts.iter().find(|a| &a.url == u),
        None => installed_kind
            .and_then(|(k, arch)| {
                report
                    .artifacts
                    .iter()
                    .find(|a| a.os == state.host.os && a.kind == k && a.arch == arch)
            })
            .or_else(|| recommend(&report.artifacts, &state.host, settings.linux_format)),
    }
    .cloned()
    .ok_or("There is no file for this system.")?;

    {
        let mut jobs = state.jobs.lock().unwrap();
        if jobs.get(&project_id).is_some_and(|j| busy(&j.view.phase)) {
            return Err("Already working on this one.".into());
        }
        jobs.insert(
            project_id.clone(),
            Job::new(
                &project_id,
                "install",
                &project.title,
                &artifact.filename,
                Phase::Queued,
                artifact.size,
            ),
        );
    }

    tauri::async_runtime::spawn(async move {
        let id = project_id.clone();
        let result = run_install(&app, &state, &project, &artifact, &settings).await;
        match result {
            Ok(saved_to) => state.update_job(&app, &id, |j| {
                j.phase = Phase::Done;
                j.saved_to = saved_to;
                j.step = None;
            }),
            Err((cancelled, e)) => state.update_job(&app, &id, |j| {
                j.phase = if cancelled {
                    Phase::Cancelled
                } else {
                    Phase::Failed
                };
                j.error = (!cancelled).then(|| e.message.clone());
                j.hint = e.hint.clone();
            }),
        }
        state.rescan().await;
        let _ = app.emit("library", ());
    });
    Ok(())
}

pub fn start_download(
    app: AppHandle,
    state: Shared,
    project_id: String,
    url: Option<String>,
) -> Result<(), String> {
    let project = state.project(&project_id).ok_or("Unknown project.")?;
    let report = state
        .report(&project_id)
        .ok_or("No downloads found for this project yet.")?;
    let settings = state.settings();
    let artifact = match &url {
        Some(u) => report.artifacts.iter().find(|a| &a.url == u),
        None => recommend(&report.artifacts, &state.host, settings.linux_format),
    }
    .cloned()
    .ok_or("There is no file for this system.")?;

    {
        let mut jobs = state.jobs.lock().unwrap();
        if jobs.get(&project_id).is_some_and(|j| busy(&j.view.phase)) {
            return Err("Already working on this one.".into());
        }
        jobs.insert(
            project_id.clone(),
            Job::new(
                &project_id,
                "download",
                &project.title,
                &artifact.filename,
                Phase::Queued,
                artifact.size,
            ),
        );
    }

    tauri::async_runtime::spawn(async move {
        let id = project_id.clone();
        let dest = user_downloads().join(&artifact.filename);
        let (stop, _) = {
            let jobs = state.jobs.lock().unwrap();
            let j = jobs.get(&id).expect("job was just added");
            (j.stop.clone(), j.cancel.clone())
        };
        state.update_job(&app, &id, |j| {
            j.phase = Phase::Downloading;
            j.step = Some("Saving to Downloads/ZLauncher".into());
        });
        let result = zl_download::download(
            &state.client,
            zl_download::Request {
                url: &artifact.url,
                dest: &dest,
                sha256: artifact.sha256.as_deref(),
            },
            &stop,
            |p| {
                state.update_job(&app, &id, |j| {
                    j.done = p.done;
                    j.total = p.total.or(j.total);
                    j.speed = p.speed;
                })
            },
        )
        .await;
        match result {
            Ok(file) => state.update_job(&app, &id, |j| {
                j.phase = Phase::Done;
                j.saved_to = Some(file.display().to_string());
                j.step = Some("Downloaded".into());
                let _ = tauri_plugin_opener::OpenerExt::opener(&app).reveal_item_in_dir(&file);
            }),
            Err(e) => {
                let cancelled = matches!(e, zl_download::Error::Cancelled);
                let e = zl_install::Error::new(e.to_string());
                state.update_job(&app, &id, |j| {
                    j.phase = if cancelled {
                        Phase::Cancelled
                    } else {
                        Phase::Failed
                    };
                    j.error = (!cancelled).then(|| e.message.clone());
                    j.hint = e.hint.clone();
                });
            }
        }
    });
    Ok(())
}

pub(crate) type JobErr = (bool, zl_install::Error);

async fn run_install(
    app: &AppHandle,
    state: &Shared,
    project: &Project,
    artifact: &Artifact,
    settings: &Settings,
) -> Result<Option<String>, JobErr> {
    let id = project.id.as_str();
    let can_install = installable(artifact.kind, Os::current());
    let dir = if can_install {
        state.downloads.join(id)
    } else {
        user_downloads()
    };
    let dest = dir.join(&artifact.filename);
    let (stop, cancel) = {
        let jobs = state.jobs.lock().unwrap();
        let j = jobs.get(id).expect("job was just added");
        (j.stop.clone(), j.cancel.clone())
    };

    state.update_job(app, id, |j| j.phase = Phase::Downloading);
    let file = zl_download::download(
        &state.client,
        zl_download::Request {
            url: &artifact.url,
            dest: &dest,
            sha256: artifact.sha256.as_deref(),
        },
        &stop,
        |p| {
            state.update_job(app, id, |j| {
                j.done = p.done;
                j.total = p.total.or(j.total);
                j.speed = p.speed;
            })
        },
    )
    .await
    .map_err(|e| {
        (
            matches!(e, zl_download::Error::Cancelled),
            zl_install::Error::new(e.to_string()),
        )
    })?;

    if !can_install {
        let _ = tauri_plugin_opener::OpenerExt::opener(app).reveal_item_in_dir(&file);
        return Ok(Some(file.display().to_string()));
    }

    install_file(app, state, project, artifact, &file, settings, cancel).await?;
    if settings.delete_after_install {
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir(&dir);
    }
    Ok(None)
}

pub(crate) type JobResult<T> = Result<T, JobErr>;

pub(crate) async fn install_file(
    app: &AppHandle,
    state: &Shared,
    project: &Project,
    artifact: &Artifact,
    file: &std::path::Path,
    settings: &Settings,
    cancel: Arc<AtomicBool>,
) -> JobResult<()> {
    let id = project.id.as_str();
    state.update_job(app, id, |j| j.phase = Phase::Installing);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let forward = {
        let (app, state, id) = (app.clone(), state.clone(), id.to_string());
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                state.update_job(&app, &id, |j| match ev {
                    zl_install::Event::Step { text } => j.step = Some(text),
                    zl_install::Event::Log { line } => {
                        j.log.push(line);
                        if j.log.len() > 400 {
                            j.log.drain(..100);
                        }
                    }
                    zl_install::Event::NeedsAuth => j.needs_auth = true,
                });
            }
        })
    };
    let options = zl_install::Options {
        menu_entry: true,
        desktop_shortcut: settings.desktop_shortcut,
    };
    let installed = zl_install::install(
        zl_install::Request {
            file,
            artifact,
            title: &project.title,
            options,
        },
        tx,
        cancel.clone(),
    )
    .await;
    let _ = forward.await;
    let installed = installed.map_err(|e| (cancel.load(Ordering::SeqCst), e))?;

    state
        .store
        .save_install(&Install {
            project_id: id.to_string(),
            version: installed.version.or(artifact.version.clone()),
            filename: artifact.filename.clone(),
            method: installed.method,
            handle: installed.handle,
            installed_at: now(),
        })
        .map_err(|e| (false, zl_install::Error::new(e.to_string())))?;
    Ok(())
}

pub fn start_uninstall(app: AppHandle, state: Shared, project_id: String) -> Result<(), String> {
    let install = state
        .store
        .install(&project_id)
        .ok()
        .flatten()
        .ok_or("This tool wasn't installed by the launcher.")?;
    let title = state
        .project(&project_id)
        .map(|p| p.title)
        .unwrap_or_else(|| project_id.clone());
    {
        let mut jobs = state.jobs.lock().unwrap();
        if jobs.get(&project_id).is_some_and(|j| busy(&j.view.phase)) {
            return Err("Already working on this one.".into());
        }
        jobs.insert(
            project_id.clone(),
            Job::new(
                &project_id,
                "uninstall",
                &title,
                &install.filename,
                Phase::Removing,
                None,
            ),
        );
    }
    tauri::async_runtime::spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let forward = {
            let (app, state, id) = (app.clone(), state.clone(), project_id.clone());
            tokio::spawn(async move {
                while let Some(ev) = rx.recv().await {
                    state.update_job(&app, &id, |j| match ev {
                        zl_install::Event::Step { text } => j.step = Some(text),
                        zl_install::Event::Log { line } => j.log.push(line),
                        zl_install::Event::NeedsAuth => j.needs_auth = true,
                    });
                }
            })
        };
        let result = zl_install::uninstall(&install.method, &install.handle, tx).await;
        let _ = forward.await;
        match result {
            Ok(()) => {
                let _ = state.store.remove_install(&project_id);
                state.update_job(&app, &project_id, |j| {
                    j.phase = Phase::Done;
                    j.step = Some("Removed".into());
                });
            }
            Err(e) => state.update_job(&app, &project_id, |j| {
                j.phase = Phase::Failed;
                j.error = Some(e.message.clone());
                j.hint = e.hint.clone();
            }),
        }
        state.rescan().await;
        let _ = app.emit("library", ());
    });
    Ok(())
}
