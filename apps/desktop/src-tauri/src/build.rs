use std::sync::atomic::Ordering;

use tauri::{AppHandle, Emitter};
use zl_catalog::Project;
use zl_crawler::{classify, Artifact, Source};

use crate::app::{busy, install_file, Job, JobErr, JobResult, Phase, Shared};

fn err(e: zl_build::Error) -> JobErr {
    (e.is_cancelled(), zl_install::Error { message: e.message, hint: e.hint })
}

pub fn requirements(state: &Shared) -> Vec<zl_build::Missing> {
    zl_build::requirements(None, state.host.linux_family.as_deref())
}

pub fn start(app: AppHandle, state: Shared, project_id: String) -> Result<(), String> {
    let project = state.project(&project_id).ok_or("Unknown project.")?;
    let (owner, repo) = project.github_repo().ok_or("This tool has no GitHub repository to build from.")?;
    {
        let mut jobs = state.jobs.lock().unwrap();
        if jobs.get(&project_id).is_some_and(|j| busy(&j.view.phase)) {
            return Err("Already working on this one.".into());
        }
        jobs.insert(project_id.clone(), Job::new(&project_id, &project.title, &format!("{owner}/{repo}"), Phase::Building, None));
    }
    tauri::async_runtime::spawn(async move {
        let id = project_id.clone();
        let result = run(&app, &state, &project, &owner, &repo).await;
        match result {
            Ok(()) => state.update_job(&app, &id, |j| {
                j.phase = Phase::Done;
                j.step = Some("Built and installed".into());
            }),
            Err((cancelled, e)) => state.update_job(&app, &id, |j| {
                j.phase = if cancelled { Phase::Cancelled } else { Phase::Failed };
                j.error = (!cancelled).then(|| e.message.clone());
                j.hint = e.hint.clone();
            }),
        }
        state.rescan().await;
        let _ = app.emit("library", ());
    });
    Ok(())
}

async fn run(app: &AppHandle, state: &Shared, project: &Project, owner: &str, repo: &str) -> JobResult<()> {
    let id = project.id.as_str();
    let cancel = state.jobs.lock().unwrap().get(id).map(|j| j.cancel.clone()).expect("job was just added");
    let fam = state.host.linux_family.clone();
    let step = |text: &str| state.update_job(app, id, |j| j.step = Some(text.to_string()));

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let forward = {
        let (app, state, id) = (app.clone(), state.clone(), id.to_string());
        tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                state.update_job(&app, &id, |j| {
                    j.log.push(line);
                    if j.log.len() > 600 {
                        j.log.drain(..150);
                    }
                });
            }
        })
    };

    let result: JobResult<(Artifact, std::path::PathBuf)> = async {
        step("Checking the build tools");
        let missing = zl_build::requirements(None, fam.as_deref());
        if !missing.is_empty() {
            let n = missing.len();
            state.update_job(app, id, |j| j.missing = missing);
            return Err((false, zl_install::Error::hint(format!("{n} build tool(s) are missing."), "Install them with the commands shown, then try again.")));
        }

        step("Downloading the source");
        let version = state.report(id).and_then(|r| r.version);
        let src = state.builds.join("src").join(repo);
        let tag = zl_build::fetch_source(owner, repo, version.as_deref(), &src, &tx, &cancel).await.map_err(err)?;
        state.update_job(app, id, |j| j.built_from = Some(tag.clone()));

        let detected = zl_build::detect(&src).map_err(err)?;
        let missing = zl_build::requirements(Some(&detected), fam.as_deref());
        if !missing.is_empty() {
            let n = missing.len();
            state.update_job(app, id, |j| j.missing = missing);
            return Err((false, zl_install::Error::hint(format!("{n} build tool(s) are missing."), "Install them with the commands shown, then try again.")));
        }

        let bundle = zl_build::Bundle::for_host(fam.as_deref());
        step(&format!("Compiling {} (a few minutes the first time)", project.title));
        let files = zl_build::build(&detected, bundle, &state.builds.join("target").join(repo), &tx, cancel.clone()).await.map_err(err)?;
        let file = files.into_iter().next().expect("build returns at least one file");
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let c = classify(&name).ok_or_else(|| (false, zl_install::Error::new(format!("The build made {name}, which the launcher can't install."))))?;
        state.update_job(app, id, |j| j.filename = name.clone());
        let artifact = Artifact {
            project_id: id.to_string(),
            url: format!("file://{}", file.display()),
            filename: name,
            os: c.os,
            arch: c.arch,
            kind: c.kind,
            version: c.version.or(detected.version),
            size: std::fs::metadata(&file).ok().map(|m| m.len()),
            sha256: None,
            source: Source::Link,
        };
        Ok((artifact, file))
    }
    .await;
    drop(tx);
    let _ = forward.await;
    let (artifact, file) = result?;
    if cancel.load(Ordering::SeqCst) {
        return Err((true, zl_install::Error::new("Stopped.")));
    }
    install_file(app, state, project, &artifact, &file, &state.settings(), cancel).await
}

pub fn cache_size(state: &Shared) -> u64 {
    fn walk(p: &std::path::Path) -> u64 {
        let Ok(rd) = std::fs::read_dir(p) else { return 0 };
        rd.flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => walk(&e.path()),
                Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => 0,
            })
            .sum()
    }
    walk(&state.builds)
}

pub fn clear_cache(state: &Shared) -> Result<(), String> {
    if state.jobs.lock().unwrap().values().any(|j| j.view.phase == Phase::Building) {
        return Err("A build is running.".into());
    }
    match std::fs::remove_dir_all(&state.builds) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
