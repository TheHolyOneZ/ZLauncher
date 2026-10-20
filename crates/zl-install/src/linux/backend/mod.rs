pub mod arch;
pub mod debian;
pub mod rpm_family;
pub mod shortcuts;

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::linux::convert::depmap::{DepMap, Verifier};
use crate::linux::distro::{Family, SystemInfo};
use crate::linux::job::{JobContext, JobError, JobResult};
use crate::linux::manage::Sink;
use crate::linux::model::{
    DepResolution, DepStatus, InstallOptions, InstallPlan, InstalledEntry, NativeFormat,
    PackageKind, PackageModel, Stage, StageId, Strategy,
};
use crate::linux::proc::Output;
use crate::linux::settings::Settings;
use crate::linux::util::arch_compatible;

pub fn strategy_for(family: Family, kind: PackageKind) -> Strategy {
    match (family, kind) {
        (_, PackageKind::AppImage) => Strategy::AppImageIntegrate,
        (Family::Arch, _) => Strategy::ConvertToNative { target: NativeFormat::Pacman },
        (Family::Debian, PackageKind::Deb) => Strategy::NativeInstall { format: NativeFormat::Deb },
        (Family::Debian, PackageKind::Rpm) => Strategy::ConvertToNative { target: NativeFormat::Deb },
        (Family::Fedora | Family::Suse, PackageKind::Rpm) => Strategy::NativeInstall { format: NativeFormat::Rpm },
        (Family::Fedora | Family::Suse, PackageKind::Deb) => Strategy::ConvertToNative { target: NativeFormat::Rpm },
        (Family::Unknown, k) => Strategy::Unsupported {
            reason: format!("ZLauncher doesn't know how to install {} files on this distribution yet. AppImages still work.", k.label()),
        },
    }
}

pub fn stages_for(strategy: &Strategy) -> Vec<Stage> {
    let ids: &[StageId] = match strategy {
        Strategy::AppImageIntegrate => &[StageId::Inspect, StageId::Move, StageId::Shortcuts],
        Strategy::NativeInstall { .. } => &[StageId::Inspect, StageId::Install, StageId::Shortcuts],
        Strategy::ConvertToNative { .. } => &[
            StageId::Inspect,
            StageId::Translate,
            StageId::Build,
            StageId::Install,
            StageId::Shortcuts,
        ],
        Strategy::Unsupported { .. } => &[StageId::Inspect],
    };
    ids.iter().map(|id| Stage::new(*id)).collect()
}

pub fn plan(model: &PackageModel, system: &SystemInfo, verifier: &dyn Verifier) -> InstallPlan {
    let mut warnings = Vec::new();
    let mut strategy = strategy_for(system.family, model.kind);

    if !arch_compatible(&model.arch, &system.arch) {
        strategy = Strategy::Unsupported {
            reason: format!(
                "This package was built for {} but this computer is {}. It can't run here.",
                model.arch, system.arch
            ),
        };
    }

    let dependencies: Vec<DepResolution> = match &strategy {
        Strategy::ConvertToNative { .. } => {
            DepMap::builtin().resolve_all(&model.dependencies, model.kind, system.family, verifier)
        }
        Strategy::NativeInstall { .. } => model
            .dependencies
            .iter()
            .map(|d| DepResolution {
                raw: d.name.clone(),
                resolved: None,
                status: DepStatus::Skipped {
                    reason: "resolved by the package manager".into(),
                },
            })
            .collect(),
        _ => Vec::new(),
    };

    let unresolved = dependencies
        .iter()
        .filter(|d| matches!(d.status, DepStatus::Unresolved))
        .count();
    if unresolved > 0 {
        warnings.push(format!(
            "{unresolved} dependenc{} couldn't be translated for {}. Most apps bundle what they need, so the install will go ahead without {}.",
            if unresolved == 1 { "y" } else { "ies" },
            system.family.label(),
            if unresolved == 1 { "it" } else { "them" }
        ));
    }

    let required_for: &[&str] = match &strategy {
        Strategy::ConvertToNative { .. } => &["convert", "install"],
        Strategy::NativeInstall { .. } => &["install"],
        _ => &[],
    };
    let missing_tools = system.missing_required(required_for);

    if matches!(strategy, Strategy::ConvertToNative { .. }) && model.kind != PackageKind::AppImage {
        warnings.push("Setup scripts bundled inside the package are not run when converting. Most desktop apps don't need them.".into());
    }
    if matches!(strategy, Strategy::AppImageIntegrate) && !system.has_tool("libfuse.so.2") {
        warnings.push("libfuse2 isn't installed, so the AppImage will be launched in extract-and-run mode (slower start-up). Install it from Settings → Tools to fix that.".into());
    }
    if model.desktop_entries.is_empty() && !matches!(strategy, Strategy::Unsupported { .. }) {
        warnings.push("This package has no menu entry of its own; ZLauncher will create one if it can find the program.".into());
    }

    let needs_root = !matches!(
        strategy,
        Strategy::AppImageIntegrate | Strategy::Unsupported { .. }
    ) && !system.is_root;

    let update_of = crate::linux::update::find_match(model, &crate::linux::registry::Registry::load());
    if let Some(u) = &update_of {
        match u.relation {
            crate::linux::version::Relation::Older => warnings.push(format!(
                "This is older than the installed {} of {} — installing it would downgrade.",
                u.installed_version, u.entry_name
            )),
            crate::linux::version::Relation::Same => warnings.push(format!(
                "{} {} is already installed; this will reinstall it.",
                u.entry_name, u.installed_version
            )),
            _ => {}
        }
    }

    InstallPlan {
        package: model.clone(),
        host: system.family,
        stages: stages_for(&strategy),
        strategy,
        dependencies,
        needs_root,
        missing_tools,
        warnings,
        update_of,
    }
}

pub async fn install(ctx: &JobContext) -> JobResult<InstalledEntry> {
    match ctx.system.family {
        Family::Arch => arch::install(ctx).await,
        Family::Debian => debian::install(ctx).await,
        Family::Fedora | Family::Suse => rpm_family::install(ctx).await,
        Family::Unknown => Err(JobError::new("Unsupported distribution.")),
    }
}

pub async fn uninstall(entry: &InstalledEntry, system: &SystemInfo, sink: &Sink) -> JobResult<()> {
    match system.family {
        Family::Arch => arch::uninstall(entry, system, sink).await,
        Family::Debian => debian::uninstall(entry, system, sink).await,
        Family::Fedora | Family::Suse => rpm_family::uninstall(entry, system, sink).await,
        Family::Unknown => Err(JobError::new(
            "Package removal isn't supported on this distribution.",
        )),
    }
}

pub async fn repair(entry: &InstalledEntry, sink: &Sink) -> JobResult<InstalledEntry> {
    let launcher = entry
        .launcher
        .iter()
        .chain(entry.desktop_files.iter())
        .find(|p| p.exists())
        .cloned()
        .ok_or_else(|| {
            JobError::with_hint(
                "No menu entry left to repair from.",
                "Reinstall the package to get its menu entry back.",
            )
        })?;
    let text = std::fs::read_to_string(&launcher).map_err(|e| JobError::new(e.to_string()))?;
    let desktop_dir = crate::linux::desktop::desktop_dir();
    let mut updated = entry.clone();
    let mut fixed = 0;
    for p in entry.desktop_files.iter().filter(|p| !p.exists()) {
        let written = if p.starts_with(&desktop_dir) {
            crate::linux::desktop::write_desktop_shortcut(&entry.slug, &text)
        } else {
            crate::linux::desktop::write_menu_entry(&entry.slug, &text)
        };
        match written {
            Ok(new_path) => {
                updated.desktop_files.retain(|x| x != p);
                updated.desktop_files.push(new_path);
                fixed += 1;
            }
            Err(e) => {
                let _ = sink.send(crate::linux::model::JobEvent::Log {
                    line: format!("could not re-create {}: {e}", p.display()),
                });
            }
        }
    }
    if updated
        .launcher
        .as_ref()
        .map(|l| !l.exists())
        .unwrap_or(true)
    {
        updated.launcher = Some(launcher);
    }
    let _ = sink.send(crate::linux::model::JobEvent::Narrate {
        text: if fixed == 0 {
            "All shortcuts were already in place".into()
        } else {
            format!(
                "Re-created {fixed} shortcut{}",
                if fixed == 1 { "" } else { "s" }
            )
        },
    });
    Ok(updated)
}

pub async fn run_root_simple(
    system: &SystemInfo,
    sink: &Sink,
    program: &Path,
    args: &[&str],
) -> JobResult<Output> {
    let dummy_plan = InstallPlan {
        package: PackageModel {
            kind: PackageKind::AppImage,
            source_path: Default::default(),
            file_size: 0,
            name: String::new(),
            display_name: String::new(),
            version: String::new(),
            arch: String::new(),
            summary: None,
            description: None,
            homepage: None,
            license: None,
            maintainer: None,
            installed_size: None,
            dependencies: vec![],
            desktop_entries: vec![],
            icon_data_url: None,
            file_count: 0,
            top_level_paths: vec![],
        },
        host: system.family,
        strategy: Strategy::AppImageIntegrate,
        stages: vec![],
        dependencies: vec![],
        needs_root: true,
        missing_tools: vec![],
        warnings: vec![],
        update_of: None,
    };
    let ctx = JobContext::new(
        dummy_plan,
        InstallOptions {
            menu_entry: false,
            desktop_shortcut: false,
            remove_original: false,
        },
        system.clone(),
        Settings::load(),
        sink.clone(),
        Arc::new(AtomicBool::new(false)),
    );
    let elevator = crate::linux::elevate::Elevator::for_context(&ctx)?;
    elevator.run(&ctx, program, args).await
}

pub async fn remove_packages(
    packages: &[String],
    system: &SystemInfo,
    sink: &Sink,
) -> JobResult<()> {
    if packages.is_empty() {
        return Ok(());
    }
    let (bin, mut args): (std::path::PathBuf, Vec<String>) = match system.family {
        Family::Arch => (
            crate::linux::distro::which("pacman")
                .ok_or_else(|| JobError::new("pacman isn't installed?!"))?,
            vec!["-R".into(), "--noconfirm".into()],
        ),
        Family::Debian => {
            let apt = crate::linux::distro::which("apt-get")
                .ok_or_else(|| JobError::new("apt-get isn't installed?!"))?;
            (
                crate::linux::distro::which("env").unwrap_or_else(|| "/usr/bin/env".into()),
                vec![
                    "DEBIAN_FRONTEND=noninteractive".into(),
                    apt.to_string_lossy().into_owned(),
                    "remove".into(),
                    "-y".into(),
                ],
            )
        }
        Family::Fedora => (
            crate::linux::distro::which("dnf")
                .or_else(|| crate::linux::distro::which("dnf5"))
                .ok_or_else(|| JobError::new("dnf isn't installed?!"))?,
            vec!["remove".into(), "-y".into()],
        ),
        Family::Suse => (
            crate::linux::distro::which("zypper")
                .ok_or_else(|| JobError::new("zypper isn't installed?!"))?,
            vec!["--non-interactive".into(), "remove".into()],
        ),
        Family::Unknown => {
            return Err(JobError::new(
                "Package removal isn't supported on this distribution.",
            ))
        }
    };
    args.push("--".into());
    args.extend(packages.iter().cloned());
    let _ = sink.send(crate::linux::model::JobEvent::Narrate {
        text: format!("Removing {} with administrator rights", packages.join(", ")),
    });
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run_root_simple(system, sink, &bin, &arg_refs).await?;
    if !out.success {
        return Err(JobError::new(format!(
            "The package manager couldn't remove {} (exit {}). See the log for details.",
            packages.join(", "),
            out.code.unwrap_or(-1)
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix() {
        assert!(matches!(
            strategy_for(Family::Arch, PackageKind::Deb),
            Strategy::ConvertToNative {
                target: NativeFormat::Pacman
            }
        ));
        assert!(matches!(
            strategy_for(Family::Debian, PackageKind::Deb),
            Strategy::NativeInstall {
                format: NativeFormat::Deb
            }
        ));
        assert!(matches!(
            strategy_for(Family::Debian, PackageKind::Rpm),
            Strategy::ConvertToNative {
                target: NativeFormat::Deb
            }
        ));
        assert!(matches!(
            strategy_for(Family::Fedora, PackageKind::Deb),
            Strategy::ConvertToNative {
                target: NativeFormat::Rpm
            }
        ));
        assert!(matches!(
            strategy_for(Family::Fedora, PackageKind::AppImage),
            Strategy::AppImageIntegrate
        ));
        assert!(matches!(
            strategy_for(Family::Unknown, PackageKind::Rpm),
            Strategy::Unsupported { .. }
        ));
    }
}
