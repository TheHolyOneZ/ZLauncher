use std::path::Path;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::linux::job::JobContext;

#[derive(Debug, Clone)]
pub struct Output {
    pub success: bool,
    pub code: Option<i32>,

    pub tail: Vec<String>,
}

impl Output {
    pub fn tail_text(&self) -> String {
        self.tail.join("\n")
    }
}

pub fn quiet_env(cmd: &mut Command) {
    cmd.env("DEBIAN_FRONTEND", "noninteractive")
        .env("LC_ALL", "C.UTF-8")
        .env("LANG", "C.UTF-8")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .stdin(Stdio::null());
}

pub async fn run_streamed(
    ctx: &JobContext,
    program: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    extra_env: &[(&str, &str)],
) -> std::io::Result<Output> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    quiet_env(&mut cmd);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    ctx.log(&format!("$ {} {}", program.display(), args.join(" ")));

    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    let tx2 = tx.clone();
    let out_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            let _ = tx2.send(l);
        }
    });
    let err_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            let _ = tx.send(l);
        }
    });

    let mut tail: Vec<String> = Vec::new();
    let mut poll = tokio::time::interval(std::time::Duration::from_millis(250));
    let status = loop {
        tokio::select! {
            Some(line) = rx.recv() => {
                ctx.log(&line);
                tail.push(line);
                if tail.len() > 40 { tail.remove(0); }
            }
            s = child.wait() => break s?,
            _ = poll.tick() => {
                if ctx.is_cancelled() {
                    ctx.log("cancelled — stopping the running command");
                    let _ = child.start_kill();
                }
            }
        }
    };
    let _ = tokio::join!(out_task, err_task);
    while let Ok(line) = rx.try_recv() {
        ctx.log(&line);
        tail.push(line);
        if tail.len() > 40 {
            tail.remove(0);
        }
    }

    Ok(Output {
        success: status.success(),
        code: status.code(),
        tail,
    })
}

pub fn run_quietly(program: &Path, args: &[&str]) -> bool {
    std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
