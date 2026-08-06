//! Persistent schedule declaration mutations.

use super::native::{print_unregister_outcome, unregister_locked};
use super::*;

/// Validate and append one schedule declaration to workspace.toml.
pub(super) fn add(arguments: ScheduleAddArgs, context: &CommandContext<'_>) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    context.verify_apply_revision(&root)?;
    let mut manifest = workspace::read(&root)?;
    if manifest
        .schedules
        .iter()
        .any(|schedule| schedule.name == arguments.name)
    {
        bail!(
            "schedule already exists: {}; use schedule update",
            arguments.name
        );
    }
    let scope = normalized_scope(&manifest, arguments.all, &arguments.repositories)?;
    let name = arguments.name;
    let schedule = ScheduleRecord {
        name: name.clone(),
        enabled: !arguments.disabled,
        action: convert_action(arguments.action),
        at: arguments.at,
        every: arguments.every,
        cron: arguments.cron,
        timezone: "local".to_owned(),
        overlap: convert_overlap(arguments.overlap),
        scope,
    };
    let output = json!({"status": "added", "schedule": &schedule});
    manifest.schedules.push(schedule);
    workspace::write(&root, &mut manifest)?;
    if context.automation.is_machine() {
        context.emit_data("add", &root, 0, &output)?;
    } else {
        println!("schedule {name} added");
        println!("next: batch-git schedule plan {name}");
        println!("then: batch-git schedule register {name}");
    }
    Ok(0)
}

/// Update only explicitly supplied declaration fields.
pub(super) fn update(arguments: ScheduleUpdateArgs, context: &CommandContext<'_>) -> Result<i32> {
    let has_trigger =
        arguments.at.is_some() || arguments.every.is_some() || arguments.cron.is_some();
    let has_scope = arguments.all || !arguments.repositories.is_empty();
    let has_changes = has_trigger
        || has_scope
        || arguments.action.is_some()
        || arguments.overlap.is_some()
        || arguments.enable
        || arguments.disable;
    if !has_changes {
        bail!("schedule update requires at least one change");
    }

    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    context.verify_apply_revision(&root)?;
    let mut manifest = workspace::read(&root)?;
    let registered = load_state(&root, &arguments.name)?.is_some();
    let scope = has_scope
        .then(|| normalized_scope(&manifest, arguments.all, &arguments.repositories))
        .transpose()?;
    let schedule = manifest
        .schedules
        .iter_mut()
        .find(|schedule| schedule.name == arguments.name)
        .ok_or_else(|| anyhow::anyhow!("unknown schedule: {}", arguments.name))?;
    if has_trigger {
        schedule.at = arguments.at;
        schedule.every = arguments.every;
        schedule.cron = arguments.cron;
    }
    if let Some(action) = arguments.action {
        schedule.action = convert_action(action);
    }
    if let Some(scope) = scope {
        schedule.scope = scope;
    }
    if let Some(overlap) = arguments.overlap {
        schedule.overlap = convert_overlap(overlap);
    }
    if arguments.enable {
        schedule.enabled = true;
    } else if arguments.disable {
        schedule.enabled = false;
    }
    let enabled = schedule.enabled;
    let updated_schedule = schedule.clone();
    workspace::write(&root, &mut manifest)?;
    if context.automation.is_machine() {
        context.emit_data(
            "update",
            &root,
            0,
            &json!({
                "status": "updated",
                "schedule": updated_schedule,
                "was_registered": registered,
                "native_update_required": registered && enabled,
            }),
        )?;
    } else {
        println!("schedule {} updated", arguments.name);
        if registered && enabled {
            println!(
                "next: batch-git schedule register {}  # apply the native task update",
                arguments.name
            );
        } else if registered {
            println!(
                "next: batch-git schedule unregister {}  # stop the disabled native task",
                arguments.name
            );
        } else {
            println!("next: batch-git schedule plan {}", arguments.name);
        }
    }
    Ok(0)
}

/// Remove a declaration after safely handling any native registration.
pub(super) fn remove(arguments: ScheduleRemoveArgs, context: &CommandContext<'_>) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    context.verify_apply_revision(&root)?;
    let mut manifest = workspace::read(&root)?;
    let Some(index) = manifest
        .schedules
        .iter()
        .position(|schedule| schedule.name == arguments.name)
    else {
        bail!("unknown schedule: {}", arguments.name);
    };
    let registered = load_state(&root, &arguments.name)?.is_some();
    if registered && !arguments.unregister {
        bail!(
            "schedule {} is registered; run schedule unregister first or use schedule remove --unregister",
            arguments.name
        );
    }
    let unregister = if arguments.unregister {
        Some(unregister_locked(
            &root,
            ScheduleUnregisterArgs {
                name: arguments.name.clone(),
                platform: SchedulePlatform::Auto,
                dry_run: false,
                purge_history: arguments.purge_history,
            },
            context.automation.is_machine(),
        )?)
    } else {
        None
    };
    manifest.schedules.remove(index);
    workspace::write(&root, &mut manifest)?;
    if context.automation.is_machine() {
        context.emit_data(
            "remove",
            &root,
            0,
            &json!({
                "status": "removed",
                "schedule": arguments.name,
                "unregistered": unregister,
                "purged_history": arguments.purge_history,
            }),
        )?;
    } else {
        if let Some(unregister) = &unregister {
            print_unregister_outcome(unregister);
        }
        println!("schedule {} removed", arguments.name);
    }
    Ok(0)
}

/// Resolve user-facing selectors to stable names before persisting the declaration.
fn normalized_scope(
    manifest: &Workspace,
    all: bool,
    repositories: &[String],
) -> Result<ScheduleScope> {
    let selected = crate::selector::select(manifest, repositories, &[], all)?;
    Ok(ScheduleScope {
        all,
        repositories: if all {
            Vec::new()
        } else {
            selected
                .into_iter()
                .map(|repository| repository.name)
                .collect()
        },
    })
}

fn convert_overlap(overlap: ScheduleOverlapValue) -> ScheduleOverlap {
    match overlap {
        ScheduleOverlapValue::Skip => ScheduleOverlap::Skip,
        ScheduleOverlapValue::Queue => ScheduleOverlap::Queue,
    }
}

fn convert_action(action: ScheduleActionValue) -> ScheduleAction {
    match action {
        ScheduleActionValue::Sync => ScheduleAction::Sync,
        ScheduleActionValue::Pull => ScheduleAction::Pull,
    }
}
