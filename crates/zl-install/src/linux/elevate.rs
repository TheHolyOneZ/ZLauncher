use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::linux::distro::which;
use crate::linux::job::{JobContext, JobError, JobResult};
use crate::linux::proc::{run_streamed, Output};
use crate::linux::settings::PrivilegeMode;

pub enum Elevator {
    Direct,

    Sudo {
        sudo: PathBuf,
        pkexec: Option<PathBuf>,
    },
    Pkexec(PathBuf),
}

pub fn sudo_path() -> Option<PathBuf> {
    which("sudo")
}

fn sudo_cmd(sudo: &Path) -> Command {
    let mut c = Command::new(sudo);
    c.env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    c
}

pub fn session_unlocked() -> bool {
    let Some(sudo) = sudo_path() else {
        return false;
    };
    sudo_cmd(&sudo)
        .args(["-n", "-v"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn session_refresh() -> bool {
    session_unlocked()
}

pub fn session_lock() {
    if let Some(sudo) = sudo_path() {
        let _ = sudo_cmd(&sudo).arg("-k").status();
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UnlockError {
    WrongPassword,

    NotAllowed { message: String },

    Unavailable,
    Io { message: String },
}

impl std::fmt::Display for UnlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnlockError::WrongPassword => write!(f, "Wrong password."),
            UnlockError::NotAllowed { message } => write!(f, "{message}"),
            UnlockError::Unavailable => write!(f, "sudo isn't installed."),
            UnlockError::Io { message } => write!(f, "{message}"),
        }
    }
}

pub fn session_unlock(password: &str) -> Result<(), UnlockError> {
    let sudo = sudo_path().ok_or(UnlockError::Unavailable)?;
    let mut child = Command::new(&sudo)
        .args(["-S", "-v", "-p", ""])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| UnlockError::Io {
            message: e.to_string(),
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(password.as_bytes());
        let _ = stdin.write_all(b"\n");
    }
    let out = child.wait_with_output().map_err(|e| UnlockError::Io {
        message: e.to_string(),
    })?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    if err.contains("try again") || err.contains("incorrect password") {
        Err(UnlockError::WrongPassword)
    } else if err.contains("not in the sudoers")
        || err.contains("not allowed")
        || err.contains("must have a tty")
        || err.contains("no tty present")
        || err.contains("askpass")
    {
        Err(UnlockError::NotAllowed { message: "Your user can't use sudo here, so ZLauncher will use the system password prompt instead.".into() })
    } else {
        Err(UnlockError::NotAllowed {
            message: format!("sudo said: {}", err.trim()),
        })
    }
}

impl Elevator {
    pub fn for_context(ctx: &JobContext) -> JobResult<Elevator> {
        if ctx.system.is_root {
            return Ok(Elevator::Direct);
        }
        let pkexec = which("pkexec");
        if ctx.settings.privilege_mode == PrivilegeMode::Session {
            if let Some(sudo) = sudo_path() {
                if session_unlocked() {
                    return Ok(Elevator::Sudo { sudo, pkexec });
                }
            }
        }
        pkexec.map(Elevator::Pkexec).ok_or_else(|| {
            JobError::with_hint(
                "Neither an unlocked sudo session nor pkexec is available, so ZLauncher can't ask for your password.",
                "Unlock the session from the lock icon in the title bar, or install polkit.",
            )
        })
    }

    pub fn describe(&self, action: &str) -> String {
        match self {
            Elevator::Direct => format!("Running as root to {action}"),
            Elevator::Sudo { .. } => format!("Using your unlocked session to {action}"),
            Elevator::Pkexec(_) => format!("Asking for your password to {action}"),
        }
    }

    fn map_pkexec(out: Output) -> JobResult<Output> {
        match (out.success, out.code) {
            (true, _) => Ok(out),
            (false, Some(126)) => Err(JobError::new("The password prompt was cancelled.")),
            (false, Some(127)) => Err(JobError::with_hint("Authorisation failed.", "Your user needs to be allowed to administer this computer (e.g. be in the wheel or sudo group).")),
            (false, _) => Ok(out),
        }
    }

    async fn via_pkexec(
        ctx: &JobContext,
        pkexec: &Path,
        program: &Path,
        args: &[&str],
    ) -> JobResult<Output> {
        ctx.needs_auth();
        let prog = program.to_string_lossy().into_owned();
        let mut full: Vec<&str> = vec![prog.as_str()];
        full.extend_from_slice(args);
        Self::map_pkexec(run_streamed(ctx, pkexec, &full, None, &[]).await?)
    }

    pub async fn run(&self, ctx: &JobContext, program: &Path, args: &[&str]) -> JobResult<Output> {
        match self {
            Elevator::Direct => Ok(run_streamed(ctx, program, args, None, &[]).await?),
            Elevator::Pkexec(pkexec) => Self::via_pkexec(ctx, pkexec, program, args).await,
            Elevator::Sudo { sudo, pkexec } => {
                let prog = program.to_string_lossy().into_owned();
                let mut full: Vec<&str> = vec!["-n", "--", prog.as_str()];
                full.extend_from_slice(args);
                let out = run_streamed(ctx, sudo, &full, None, &[]).await?;
                let expired = !out.success
                    && out
                        .tail_text()
                        .to_lowercase()
                        .contains("password is required");
                if expired {
                    ctx.narrate("The unlocked session has expired — asking for your password");
                    match pkexec {
                        Some(p) => Self::via_pkexec(ctx, p, program, args).await,
                        None => Err(JobError::with_hint(
                            "The unlocked session expired.",
                            "Unlock again from the lock icon in the title bar and retry.",
                        )),
                    }
                } else {
                    Ok(out)
                }
            }
        }
    }
}
