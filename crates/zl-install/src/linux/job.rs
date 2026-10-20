use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use thiserror::Error;
use tokio::sync::mpsc;

use crate::linux::distro::SystemInfo;
use crate::linux::model::{
    InstallOptions, InstallPlan, InstalledEntry, JobEvent, StageId, StageStatus, Strategy,
};
use crate::linux::registry::Registry;
use crate::linux::settings::Settings;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct JobError {
    pub message: String,
    pub hint: Option<String>,
}

impl JobError {
    pub fn new(message: impl Into<String>) -> Self {
        JobError {
            message: message.into(),
            hint: None,
        }
    }
    pub fn with_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        JobError {
            message: message.into(),
            hint: Some(hint.into()),
        }
    }
    pub fn cancelled() -> Self {
        JobError::new("Cancelled.")
    }
}

impl From<std::io::Error> for JobError {
    fn from(e: std::io::Error) -> Self {
        JobError::new(e.to_string())
    }
}

pub type JobResult<T> = Result<T, JobError>;

#[derive(Clone)]
pub struct JobContext {
    pub plan: InstallPlan,
    pub options: InstallOptions,
    pub system: SystemInfo,
    pub settings: Settings,
    events: mpsc::UnboundedSender<JobEvent>,
    cancel: Arc<AtomicBool>,
}

impl JobContext {
    pub fn new(
        plan: InstallPlan,
        options: InstallOptions,
        system: SystemInfo,
        settings: Settings,
        events: mpsc::UnboundedSender<JobEvent>,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        JobContext {
            plan,
            options,
            system,
            settings,
            events,
            cancel,
        }
    }

    pub fn send(&self, event: JobEvent) {
        let _ = self.events.send(event);
    }
    pub fn narrate(&self, text: impl Into<String>) {
        self.send(JobEvent::Narrate { text: text.into() });
    }
    pub fn log(&self, line: &str) {
        self.send(JobEvent::Log {
            line: line.to_string(),
        });
    }
    pub fn needs_auth(&self) {
        self.send(JobEvent::NeedsAuth);
    }

    fn stage_index(&self, id: StageId) -> Option<usize> {
        self.plan.stages.iter().position(|s| s.id == id)
    }
    pub fn begin(&self, id: StageId) {
        if let Some(index) = self.stage_index(id) {
            self.send(JobEvent::Stage {
                index,
                status: StageStatus::Active,
            });
        }
    }
    pub fn finish(&self, id: StageId) {
        if let Some(index) = self.stage_index(id) {
            self.send(JobEvent::Stage {
                index,
                status: StageStatus::Done,
            });
        }
    }
    pub fn fail(&self, id: StageId) {
        if let Some(index) = self.stage_index(id) {
            self.send(JobEvent::Stage {
                index,
                status: StageStatus::Failed,
            });
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    pub fn check_cancelled(&self) -> JobResult<()> {
        if self.is_cancelled() {
            Err(JobError::cancelled())
        } else {
            Ok(())
        }
    }
}

pub async fn run(ctx: JobContext) {
    let result: JobResult<(StageId, InstalledEntry)> = async {
        match &ctx.plan.strategy {
            Strategy::AppImageIntegrate => crate::linux::appimage::integrate(&ctx)
                .await
                .map(|e| (StageId::Shortcuts, e)),
            Strategy::ConvertToNative { .. } | Strategy::NativeInstall { .. } => {
                crate::linux::backend::install(&ctx)
                    .await
                    .map(|e| (StageId::Shortcuts, e))
            }
            Strategy::Unsupported { reason } => Err(JobError::new(reason.clone())),
        }
    }
    .await;

    match result {
        Ok((_, mut entry)) => {
            let mut registry = Registry::load();

            let previous = ctx
                .plan
                .update_of
                .as_ref()
                .and_then(|u| registry.find(&u.entry_id).cloned())
                .or_else(|| {
                    registry
                        .entries
                        .iter()
                        .find(|e| e.slug == entry.slug && e.install_kind == entry.install_kind)
                        .cloned()
                });
            if let Some(prev) = previous {
                entry.id = prev.id.clone();
                entry.update_dir = prev.update_dir.clone();
                entry.ignored_versions = prev.ignored_versions.clone();
                entry.history = prev.history.clone();
                if prev.version != entry.version || prev.source_kind != entry.source_kind {
                    entry.history.push(crate::linux::model::VersionRecord {
                        version: prev.version.clone(),
                        installed_at: prev.installed_at.clone(),
                        source_kind: prev.source_kind,
                    });
                }
                registry.entries.retain(|e| e.id != prev.id);
            }
            registry.entries.retain(|e| {
                !(e.slug == entry.slug && e.install_kind == entry.install_kind && e.id != entry.id)
            });
            registry.upsert(entry.clone());
            if let Err(e) = registry.save() {
                ctx.log(&format!("warning: could not save registry: {e}"));
            }
            ctx.send(JobEvent::Done { entry });
        }
        Err(e) => {
            ctx.send(JobEvent::Failed {
                message: e.message,
                hint: e.hint,
            });
        }
    }
}
