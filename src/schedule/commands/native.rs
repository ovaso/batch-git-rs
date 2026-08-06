//! Native scheduler definition, registration, and removal workflows.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::json;

use super::CommandContext;
use super::execution::build_plan;
use super::support::find_schedule;
use crate::cli::{
    SchedulePlatform, SchedulePlatformArgs, ScheduleRegisterArgs, ScheduleUnregisterArgs,
};
use crate::model::now;
use crate::schedule::artifact::{
    artifact_digest, artifact_files_match, artifact_output, build_artifact, print_artifact,
};
use crate::schedule::registration::{
    deactivate, native_task_exists, remove_registered_files, write_and_activate,
};
use crate::schedule::state::{
    load_state, registration_state_path, schedule_log_directory, write_state,
};
use crate::schedule::{NativePlatform, RegistrationState};
use crate::workspace::{self, WorkspaceLock};

/// Generate native definitions for review without writing them.
pub(super) fn generate(
    arguments: SchedulePlatformArgs,
    context: &CommandContext<'_>,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let schedule = find_schedule(&manifest, &arguments.name)?;
    let platform = NativePlatform::resolve(arguments.platform)?;
    let artifact = build_artifact(&root, schedule, platform)?;
    if context.automation.is_machine() {
        context.emit_data(
            "generate",
            &root,
            0,
            &json!({
                "schedule": arguments.name,
                "artifact": artifact_output(&artifact),
            }),
        )?;
    } else {
        print_artifact(&artifact);
    }
    Ok(0)
}

/// Idempotently create or update a native task and atomically persist its registration summary.
pub(super) fn register(
    arguments: ScheduleRegisterArgs,
    context: &CommandContext<'_>,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    context.verify_apply_revision(&root)?;
    let manifest = workspace::read(&root)?;
    let schedule = find_schedule(&manifest, &arguments.name)?;
    if !schedule.enabled {
        bail!("schedule is disabled: {}", schedule.name);
    }
    build_plan(&manifest, &arguments.name)?;
    let platform = NativePlatform::resolve(arguments.platform)?;
    let artifact = build_artifact(&root, schedule, platform)?;
    let existing_state = load_state(&root, &arguments.name)?;
    if let Some(state) = &existing_state
        && state.platform != platform.label()
        && !arguments.migrate
    {
        bail!(
            "schedule {} is registered on {}; use --migrate to move it to {}",
            arguments.name,
            state.platform,
            platform.label()
        );
    }
    let digest = artifact_digest(&artifact);
    let unchanged = existing_state
        .as_ref()
        .is_some_and(|state| state.platform == platform.label() && state.digest == digest)
        && artifact_files_match(&artifact)?;
    if unchanged {
        if context.automation.is_machine() {
            context.emit_data(
                "register",
                &root,
                0,
                &json!({
                    "schedule": &arguments.name,
                    "status": "unchanged",
                    "platform": platform.label(),
                    "task_id": &artifact.task_id,
                    "dry_run": false,
                }),
            )?;
        } else {
            println!("schedule {} unchanged", arguments.name);
        }
        return Ok(0);
    }
    let native_collision = artifact.files.iter().any(|file| file.path.exists())
        || native_task_exists(artifact.platform, &artifact.task_id)?;
    if existing_state.is_none() && native_collision && !arguments.force {
        bail!(
            "native task already exists for {}; use --force to replace it",
            arguments.name
        );
    }
    if arguments.dry_run {
        let operation = if existing_state.is_some() {
            "update"
        } else {
            "register"
        };
        if context.automation.is_machine() {
            context.emit_data(
                "register",
                &root,
                0,
                &json!({
                    "schedule": &arguments.name,
                    "status": "planned",
                    "operation": operation,
                    "platform": platform.label(),
                    "task_id": &artifact.task_id,
                    "dry_run": true,
                    "artifact": artifact_output(&artifact),
                }),
            )?;
        } else {
            println!(
                "would {operation} schedule {} on {}",
                arguments.name,
                platform.label()
            );
            print_artifact(&artifact);
        }
        return Ok(0);
    }

    let was_registered = existing_state.is_some();
    let updating_same_platform = existing_state
        .as_ref()
        .is_some_and(|state| state.platform == platform.label());
    write_and_activate(
        &artifact,
        updating_same_platform,
        context.automation.is_machine(),
    )?;
    let timestamp = now();
    let state = RegistrationState {
        workspace: root.display().to_string(),
        name: arguments.name.clone(),
        platform: platform.label().to_owned(),
        task_id: artifact.task_id.clone(),
        digest,
        files: artifact
            .files
            .iter()
            .map(|file| file.path.display().to_string())
            .collect(),
        log_enabled: artifact.log_enabled,
        stdout_path: artifact
            .stdout_path
            .as_ref()
            .map(|path| path.display().to_string()),
        stderr_path: artifact
            .stderr_path
            .as_ref()
            .map(|path| path.display().to_string()),
        registered_at: existing_state
            .as_ref()
            .map(|state| state.registered_at.clone())
            .unwrap_or_else(|| timestamp.clone()),
        updated_at: timestamp,
    };
    if let Some(previous) = &existing_state
        && previous.platform != platform.label()
        && let Err(error) = deactivate(previous, context.automation.is_machine())
            .and_then(|()| remove_registered_files(previous, context.automation.is_machine()))
    {
        let _ = deactivate(&state, context.automation.is_machine());
        let _ = remove_registered_files(&state, context.automation.is_machine());
        return Err(error).context("failed to remove previous schedule registration");
    }
    write_state(&root, &state)?;
    let status = if was_registered {
        "updated"
    } else {
        "registered"
    };
    if context.automation.is_machine() {
        context.emit_data(
            "register",
            &root,
            0,
            &json!({
                "schedule": &arguments.name,
                "status": status,
                "platform": platform.label(),
                "task_id": &state.task_id,
                "dry_run": false,
                "logging": state.log_enabled,
                "native_files": &state.files,
            }),
        )?;
    } else {
        println!(
            "schedule {} {status} on {}",
            arguments.name,
            platform.label()
        );
        println!("verify: batch-git schedule status {}", arguments.name);
    }
    Ok(0)
}

/// Final unregister result reused by `remove` without producing a second machine receipt.
#[derive(Serialize)]
pub(super) struct UnregisterOutcome {
    schedule: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
    dry_run: bool,
    purged_history: bool,
}

pub(super) fn print_unregister_outcome(outcome: &UnregisterOutcome) {
    match outcome.status {
        "already_absent" => println!("schedule {} already absent", outcome.schedule),
        "planned" => println!(
            "would unregister schedule {} from {}",
            outcome.schedule,
            outcome.platform.as_deref().unwrap_or("-")
        ),
        "unregistered" => println!(
            "schedule {} unregistered from {}",
            outcome.schedule,
            outcome.platform.as_deref().unwrap_or("-")
        ),
        _ => unreachable!("invalid unregister outcome status"),
    }
}

/// Acquire the workspace lock before unregistering a native task.
pub(super) fn unregister(
    arguments: ScheduleUnregisterArgs,
    context: &CommandContext<'_>,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    context.verify_apply_revision(&root)?;
    let outcome = unregister_locked(&root, arguments, context.automation.is_machine())?;
    if context.automation.is_machine() {
        context.emit_data("unregister", &root, 0, &outcome)?;
    } else {
        print_unregister_outcome(&outcome);
    }
    Ok(0)
}

/// Deactivate a task and remove its definition, state, and optional history while already locked.
pub(super) fn unregister_locked(
    root: &Path,
    arguments: ScheduleUnregisterArgs,
    quiet: bool,
) -> Result<UnregisterOutcome> {
    let state = load_state(root, &arguments.name)?;
    let Some(state) = state else {
        return Ok(UnregisterOutcome {
            schedule: arguments.name,
            status: "already_absent",
            platform: None,
            dry_run: false,
            purged_history: false,
        });
    };
    let requested = match arguments.platform {
        SchedulePlatform::Auto => NativePlatform::from_label(&state.platform)?,
        platform => NativePlatform::resolve(platform)?,
    };
    if requested.label() != state.platform {
        bail!(
            "schedule {} is registered on {}, not {}",
            arguments.name,
            state.platform,
            requested.label()
        );
    }
    if arguments.dry_run {
        return Ok(UnregisterOutcome {
            schedule: arguments.name,
            status: "planned",
            platform: Some(state.platform),
            dry_run: true,
            purged_history: false,
        });
    }
    deactivate(&state, quiet)?;
    remove_registered_files(&state, quiet)?;
    let state_path = registration_state_path(root, &arguments.name)?;
    if state_path.exists() {
        fs::remove_file(&state_path)
            .with_context(|| format!("failed to remove {}", state_path.display()))?;
    }
    if arguments.purge_history {
        let logs = schedule_log_directory(root, &arguments.name)?;
        if logs.is_dir() {
            fs::remove_dir_all(&logs)
                .with_context(|| format!("failed to remove {}", logs.display()))?;
        }
    }
    Ok(UnregisterOutcome {
        schedule: arguments.name,
        status: "unregistered",
        platform: Some(state.platform),
        dry_run: false,
        purged_history: arguments.purge_history,
    })
}
