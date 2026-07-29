//! Schedule planning, execution, native definition generation, and registration.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::cli::{
    ScheduleActionValue, ScheduleAddArgs, ScheduleArgs, ScheduleCommand, ScheduleDoctorArgs,
    ScheduleListArgs, ScheduleNameArgs, ScheduleNativeRunArgs, ScheduleOverlapValue,
    SchedulePlatform, SchedulePlatformArgs, ScheduleRegisterArgs, ScheduleRemoveArgs,
    ScheduleRunArgs, ScheduleUnregisterArgs, ScheduleUpdateArgs,
};
use crate::cron::CronExpression;
use crate::model::{
    ScheduleAction, ScheduleOverlap, ScheduleRecord, ScheduleScope, Workspace, now,
    schedule_interval_seconds,
};
use crate::settings;
use crate::table;
use crate::workspace::{self, WorkspaceLock};

pub(crate) fn dispatch(arguments: ScheduleArgs, jobs: usize, verbose: bool) -> Result<i32> {
    match arguments.command {
        ScheduleCommand::Add(arguments) => add(arguments),
        ScheduleCommand::Plan(arguments) => plan(arguments),
        ScheduleCommand::Run(arguments) => run(arguments, jobs, verbose),
        ScheduleCommand::List(arguments) => list(arguments),
        ScheduleCommand::NativeRun(arguments) => native_run(arguments),
        ScheduleCommand::Status(arguments) => status(arguments),
        ScheduleCommand::Doctor(arguments) => doctor(arguments),
        ScheduleCommand::Generate(arguments) => generate(arguments),
        ScheduleCommand::Register(arguments) => register(arguments),
        ScheduleCommand::Remove(arguments) => remove(arguments),
        ScheduleCommand::Unregister(arguments) => unregister(arguments),
        ScheduleCommand::Update(arguments) => update(arguments),
    }
}

fn add(arguments: ScheduleAddArgs) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
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
    manifest.schedules.push(ScheduleRecord {
        name: name.clone(),
        enabled: !arguments.disabled,
        action: convert_action(arguments.action),
        at: arguments.at,
        every: arguments.every,
        cron: arguments.cron,
        timezone: "local".to_owned(),
        overlap: convert_overlap(arguments.overlap),
        scope,
    });
    workspace::write(&root, &mut manifest)?;
    println!("schedule {name} added");
    println!("next: batch-git schedule plan {name}");
    println!("then: batch-git schedule register {name}");
    Ok(0)
}

fn update(arguments: ScheduleUpdateArgs) -> Result<i32> {
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
    workspace::write(&root, &mut manifest)?;
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
    Ok(0)
}

fn remove(arguments: ScheduleRemoveArgs) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
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
    if arguments.unregister {
        unregister_locked(
            &root,
            ScheduleUnregisterArgs {
                name: arguments.name.clone(),
                platform: SchedulePlatform::Auto,
                dry_run: false,
                purge_history: arguments.purge_history,
            },
        )?;
    }
    manifest.schedules.remove(index);
    workspace::write(&root, &mut manifest)?;
    println!("schedule {} removed", arguments.name);
    Ok(0)
}

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

#[derive(Debug, Serialize)]
struct PlanOutput {
    schedule: String,
    action: &'static str,
    trigger: String,
    overlap: &'static str,
    repositories: Vec<String>,
}

fn plan(arguments: ScheduleNameArgs) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let output = build_plan(&manifest, &arguments.name)?;
    if arguments.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print_plan(&output);
    }
    Ok(0)
}

fn build_plan(manifest: &Workspace, name: &str) -> Result<PlanOutput> {
    let schedule = find_schedule(manifest, name)?;
    if !schedule.enabled {
        bail!("schedule is disabled: {name}");
    }
    let repositories = selected_repositories(manifest, schedule)?;
    Ok(PlanOutput {
        schedule: schedule.name.clone(),
        action: action_label(schedule.action),
        trigger: trigger_label(schedule),
        overlap: overlap_label(schedule.overlap),
        repositories: repositories
            .into_iter()
            .map(|repository| repository.name)
            .collect(),
    })
}

fn print_plan(output: &PlanOutput) {
    let rows = output
        .repositories
        .iter()
        .map(|repository| vec![repository.clone(), output.action.to_owned()])
        .collect::<Vec<_>>();
    print!("{}", table::render(&["REPOSITORY", "ACTION"], &rows));
    println!();
    println!(
        "schedule: {}; trigger: {}; overlap: {}; {} repositories",
        output.schedule,
        output.trigger,
        output.overlap,
        output.repositories.len()
    );
}

fn run(arguments: ScheduleRunArgs, jobs: usize, verbose: bool) -> Result<i32> {
    let root = workspace::find_root()?;
    let initial = workspace::read(&root)?;
    let overlap = find_schedule(&initial, &arguments.name)?.overlap;
    let _lock = match overlap {
        ScheduleOverlap::Queue => WorkspaceLock::acquire(&root)?,
        ScheduleOverlap::Skip => match WorkspaceLock::try_acquire(&root)? {
            Some(lock) => lock,
            None => {
                println!("schedule {} skipped: workspace is locked", arguments.name);
                return Ok(0);
            }
        },
    };
    let mut manifest = workspace::read(&root)?;
    let schedule = find_schedule(&manifest, &arguments.name)?.clone();
    if !schedule.enabled {
        bail!("schedule is disabled: {}", schedule.name);
    }
    let repositories = selected_repositories(&manifest, &schedule)?;
    match schedule.action {
        ScheduleAction::Sync => {
            crate::commands::run_sync(&root, &mut manifest, &repositories, jobs, verbose)
        }
        ScheduleAction::Pull => {
            crate::commands::run_pull(&root, &mut manifest, &repositories, jobs, verbose)
        }
    }
}

fn native_run(arguments: ScheduleNativeRunArgs) -> Result<i32> {
    let root = workspace::find_root()?;
    let mut command =
        Command::new(env::current_exe().context("failed to locate batch-git executable")?);
    command
        .args(["schedule", "run", &arguments.name])
        .current_dir(&root)
        .env("BATCH_GIT_WORKSPACE", &root)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null());
    if let Some(timezone) = &arguments.timezone {
        command.env("BATCH_GIT_TZ", timezone).env("TZ", timezone);
    } else {
        command.env_remove("BATCH_GIT_TZ").env_remove("TZ");
    }
    if arguments.log {
        let log_directory = schedule_log_directory(&root, &arguments.name)?;
        fs::create_dir_all(&log_directory).with_context(|| {
            format!(
                "failed to create schedule log directory {}",
                log_directory.display()
            )
        })?;
        let stdout = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_directory.join("stdout.log"))?;
        let stderr = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_directory.join("stderr.log"))?;
        command
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
    } else {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let status = command
        .status()
        .context("failed to launch scheduled batch-git run")?;
    Ok(status.code().unwrap_or(1))
}

#[derive(Serialize)]
struct ScheduleListItem {
    name: String,
    enabled: bool,
    action: &'static str,
    trigger: String,
    scope: String,
    registered: Option<String>,
}

fn list(arguments: ScheduleListArgs) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let items = manifest
        .schedules
        .iter()
        .map(|schedule| {
            let registered = if arguments.registered {
                load_state(&root, &schedule.name)
                    .ok()
                    .flatten()
                    .map(|state| state.platform)
                    .or_else(|| Some("-".to_owned()))
            } else {
                None
            };
            ScheduleListItem {
                name: schedule.name.clone(),
                enabled: schedule.enabled,
                action: action_label(schedule.action),
                trigger: trigger_label(schedule),
                scope: if schedule.scope.all {
                    "all".to_owned()
                } else {
                    schedule.scope.repositories.join(",")
                },
                registered,
            }
        })
        .collect::<Vec<_>>();
    if arguments.json {
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else {
        let mut headers = vec!["NAME", "ENABLED", "ACTION", "TRIGGER", "SCOPE"];
        if arguments.registered {
            headers.push("REGISTERED");
        }
        let rows = items
            .iter()
            .map(|item| {
                let mut row = vec![
                    item.name.clone(),
                    if item.enabled { "yes" } else { "no" }.to_owned(),
                    item.action.to_owned(),
                    item.trigger.clone(),
                    item.scope.clone(),
                ];
                if let Some(registered) = &item.registered {
                    row.push(registered.clone());
                }
                row
            })
            .collect::<Vec<_>>();
        print!("{}", table::render(&headers, &rows));
        println!();
        println!("{} schedules", items.len());
    }
    Ok(0)
}

#[derive(Serialize)]
struct StatusOutput {
    name: String,
    declared: bool,
    enabled: Option<bool>,
    trigger: Option<String>,
    registered: bool,
    platform: Option<String>,
    task_id: Option<String>,
    native_loaded: Option<bool>,
    definition_matches: Option<bool>,
    native_state: Option<String>,
    runs: Option<u64>,
    last_exit_code: Option<i32>,
    logging: Option<bool>,
    stdout: Option<String>,
    stderr: Option<String>,
    native_files: Vec<String>,
    updated_at: Option<String>,
}

fn status(arguments: ScheduleNameArgs) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let schedule = manifest
        .schedules
        .iter()
        .find(|schedule| schedule.name == arguments.name);
    let state = load_state(&root, &arguments.name)?;
    if schedule.is_none() && state.is_none() {
        bail!("unknown schedule: {}", arguments.name);
    }
    let native = state.as_ref().map(native_status).transpose()?;
    let definition_matches = match (schedule, state.as_ref()) {
        (Some(schedule), Some(state)) => {
            let platform = NativePlatform::from_label(&state.platform)?;
            let artifact =
                build_artifact_with_logging(&root, schedule, platform, state.log_enabled)?;
            Some(state.digest == artifact_digest(&artifact) && artifact_files_match(&artifact)?)
        }
        _ => None,
    };
    let output = StatusOutput {
        name: arguments.name,
        declared: schedule.is_some(),
        enabled: schedule.map(|schedule| schedule.enabled),
        trigger: schedule.map(trigger_label),
        registered: state.is_some(),
        platform: state.as_ref().map(|state| state.platform.clone()),
        task_id: state.as_ref().map(|state| state.task_id.clone()),
        native_loaded: native.as_ref().map(|status| status.loaded),
        definition_matches,
        native_state: native.as_ref().and_then(|status| status.state.clone()),
        runs: native.as_ref().and_then(|status| status.runs),
        last_exit_code: native.as_ref().and_then(|status| status.last_exit_code),
        logging: state.as_ref().map(|state| state.log_enabled),
        stdout: state.as_ref().and_then(|state| state.stdout_path.clone()),
        stderr: state.as_ref().and_then(|state| state.stderr_path.clone()),
        native_files: state
            .as_ref()
            .map(|state| state.files.clone())
            .unwrap_or_default(),
        updated_at: state.as_ref().map(|state| state.updated_at.clone()),
    };
    if arguments.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        let rows = vec![
            vec!["NAME".to_owned(), output.name],
            vec![
                "DECLARED".to_owned(),
                if output.declared { "yes" } else { "no" }.to_owned(),
            ],
            vec![
                "ENABLED".to_owned(),
                output
                    .enabled
                    .map(|value| if value { "yes" } else { "no" }.to_owned())
                    .unwrap_or_else(|| "-".to_owned()),
            ],
            vec![
                "TRIGGER".to_owned(),
                output.trigger.unwrap_or_else(|| "-".to_owned()),
            ],
            vec![
                "REGISTERED".to_owned(),
                if output.registered { "yes" } else { "no" }.to_owned(),
            ],
            vec![
                "PLATFORM".to_owned(),
                output.platform.unwrap_or_else(|| "-".to_owned()),
            ],
            vec![
                "TASK ID".to_owned(),
                output.task_id.unwrap_or_else(|| "-".to_owned()),
            ],
            vec![
                "NATIVE LOADED".to_owned(),
                output
                    .native_loaded
                    .map(yes_no)
                    .unwrap_or_else(|| "-".to_owned()),
            ],
            vec![
                "DEFINITION MATCHES".to_owned(),
                output
                    .definition_matches
                    .map(yes_no)
                    .unwrap_or_else(|| "-".to_owned()),
            ],
            vec![
                "NATIVE STATE".to_owned(),
                output.native_state.unwrap_or_else(|| "-".to_owned()),
            ],
            vec![
                "RUNS".to_owned(),
                output
                    .runs
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
            ],
            vec![
                "LAST EXIT CODE".to_owned(),
                output
                    .last_exit_code
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
            ],
            vec![
                "LOGGING".to_owned(),
                output
                    .logging
                    .map(|value| if value { "enabled" } else { "disabled" }.to_owned())
                    .unwrap_or_else(|| "-".to_owned()),
            ],
            vec![
                "STDOUT".to_owned(),
                output.stdout.unwrap_or_else(|| "-".to_owned()),
            ],
            vec![
                "STDERR".to_owned(),
                output.stderr.unwrap_or_else(|| "-".to_owned()),
            ],
            vec![
                "NATIVE FILES".to_owned(),
                if output.native_files.is_empty() {
                    "-".to_owned()
                } else {
                    output.native_files.join(", ")
                },
            ],
            vec![
                "UPDATED".to_owned(),
                output.updated_at.unwrap_or_else(|| "-".to_owned()),
            ],
        ];
        print!("{}", table::render(&["FIELD", "VALUE"], &rows));
    }
    Ok(0)
}

fn doctor(arguments: ScheduleDoctorArgs) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let platform = NativePlatform::resolve(arguments.platform)?;
    let names = match arguments.name {
        Some(name) => vec![name],
        None => manifest
            .schedules
            .iter()
            .map(|schedule| schedule.name.clone())
            .collect(),
    };
    let mut checks = Vec::new();
    for name in names {
        let plan = build_plan(&manifest, &name)?;
        let schedule = find_schedule(&manifest, &name)?;
        let artifact = build_artifact(&root, schedule, platform)?;
        checks.push(serde_json::json!({
            "schedule": name,
            "platform": platform.label(),
            "repositories": plan.repositories.len(),
            "task_id": artifact.task_id,
            "status": "ok"
        }));
    }
    if arguments.json {
        println!("{}", serde_json::to_string_pretty(&checks)?);
    } else if checks.is_empty() {
        println!("no schedules declared");
    } else {
        for check in checks {
            println!(
                "{}: ok ({}, {} repositories)",
                check["schedule"].as_str().unwrap_or("-"),
                check["platform"].as_str().unwrap_or("-"),
                check["repositories"].as_u64().unwrap_or(0)
            );
        }
    }
    Ok(0)
}

fn generate(arguments: SchedulePlatformArgs) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let schedule = find_schedule(&manifest, &arguments.name)?;
    let platform = NativePlatform::resolve(arguments.platform)?;
    let artifact = build_artifact(&root, schedule, platform)?;
    print_artifact(&artifact);
    Ok(0)
}

fn register(arguments: ScheduleRegisterArgs) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
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
        println!("schedule {} unchanged", arguments.name);
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
        println!(
            "would {} schedule {} on {}",
            if existing_state.is_some() {
                "update"
            } else {
                "register"
            },
            arguments.name,
            platform.label()
        );
        print_artifact(&artifact);
        return Ok(0);
    }

    let was_registered = existing_state.is_some();
    let updating_same_platform = existing_state
        .as_ref()
        .is_some_and(|state| state.platform == platform.label());
    write_and_activate(&artifact, updating_same_platform)?;
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
        && let Err(error) = deactivate(previous).and_then(|()| remove_registered_files(previous))
    {
        let _ = deactivate(&state);
        let _ = remove_registered_files(&state);
        return Err(error).context("failed to remove previous schedule registration");
    }
    write_state(&root, &state)?;
    println!(
        "schedule {} {} on {}",
        arguments.name,
        if was_registered {
            "updated"
        } else {
            "registered"
        },
        platform.label()
    );
    println!("verify: batch-git schedule status {}", arguments.name);
    Ok(0)
}

fn unregister(arguments: ScheduleUnregisterArgs) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    unregister_locked(&root, arguments)
}

fn unregister_locked(root: &Path, arguments: ScheduleUnregisterArgs) -> Result<i32> {
    let state = load_state(root, &arguments.name)?;
    let Some(state) = state else {
        println!("schedule {} already absent", arguments.name);
        return Ok(0);
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
        println!(
            "would unregister schedule {} from {}",
            arguments.name, state.platform
        );
        return Ok(0);
    }
    deactivate(&state)?;
    remove_registered_files(&state)?;
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
    println!(
        "schedule {} unregistered from {}",
        arguments.name, state.platform
    );
    Ok(0)
}

fn find_schedule<'a>(manifest: &'a Workspace, name: &str) -> Result<&'a ScheduleRecord> {
    manifest
        .schedules
        .iter()
        .find(|schedule| schedule.name == name)
        .ok_or_else(|| anyhow::anyhow!("unknown schedule: {name}"))
}

fn selected_repositories(
    manifest: &Workspace,
    schedule: &ScheduleRecord,
) -> Result<Vec<crate::model::RepositoryRecord>> {
    crate::selector::select(
        manifest,
        &schedule.scope.repositories,
        &[],
        schedule.scope.all,
    )
}

fn trigger_label(schedule: &ScheduleRecord) -> String {
    schedule
        .at
        .as_ref()
        .map(|at| format!("daily {at}"))
        .or_else(|| {
            schedule
                .every
                .as_ref()
                .map(|every| format!("every {every}"))
        })
        .or_else(|| schedule.cron.as_ref().map(|cron| format!("cron {cron}")))
        .unwrap_or_else(|| "invalid".to_owned())
}

fn overlap_label(overlap: ScheduleOverlap) -> &'static str {
    match overlap {
        ScheduleOverlap::Skip => "skip",
        ScheduleOverlap::Queue => "queue",
    }
}

fn action_label(action: ScheduleAction) -> &'static str {
    match action {
        ScheduleAction::Sync => "sync",
        ScheduleAction::Pull => "pull",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativePlatform {
    Launchd,
    Systemd,
    Windows,
}

impl NativePlatform {
    fn resolve(platform: SchedulePlatform) -> Result<Self> {
        match platform {
            SchedulePlatform::Launchd => Ok(Self::Launchd),
            SchedulePlatform::Systemd => Ok(Self::Systemd),
            SchedulePlatform::Windows => Ok(Self::Windows),
            SchedulePlatform::Auto => {
                #[cfg(target_os = "macos")]
                {
                    Ok(Self::Launchd)
                }
                #[cfg(target_os = "linux")]
                {
                    Ok(Self::Systemd)
                }
                #[cfg(target_os = "windows")]
                {
                    Ok(Self::Windows)
                }
                #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
                {
                    bail!("automatic schedule platform detection is unsupported on this OS")
                }
            }
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Launchd => "launchd",
            Self::Systemd => "systemd",
            Self::Windows => "windows",
        }
    }

    fn from_label(label: &str) -> Result<Self> {
        match label {
            "launchd" => Ok(Self::Launchd),
            "systemd" => Ok(Self::Systemd),
            "windows" => Ok(Self::Windows),
            _ => bail!("unsupported registered schedule platform: {label}"),
        }
    }
}

struct NativeFile {
    path: PathBuf,
    content: String,
}

struct NativeArtifact {
    platform: NativePlatform,
    task_id: String,
    log_enabled: bool,
    log_directory: Option<PathBuf>,
    stdout_path: Option<PathBuf>,
    stderr_path: Option<PathBuf>,
    files: Vec<NativeFile>,
}

fn build_artifact(
    root: &Path,
    schedule: &ScheduleRecord,
    platform: NativePlatform,
) -> Result<NativeArtifact> {
    build_artifact_with_options(
        root,
        schedule,
        platform,
        settings::schedule_log_enabled()?,
        settings::schedule_timezone()?.as_deref(),
    )
}

fn build_artifact_with_logging(
    root: &Path,
    schedule: &ScheduleRecord,
    platform: NativePlatform,
    log_enabled: bool,
) -> Result<NativeArtifact> {
    build_artifact_with_options(
        root,
        schedule,
        platform,
        log_enabled,
        settings::schedule_timezone()?.as_deref(),
    )
}

fn build_artifact_with_options(
    root: &Path,
    schedule: &ScheduleRecord,
    platform: NativePlatform,
    log_enabled: bool,
    timezone: Option<&str>,
) -> Result<NativeArtifact> {
    let executable = env::current_exe()
        .context("failed to locate batch-git executable")?
        .canonicalize()
        .context("failed to canonicalize batch-git executable")?;
    let task_id = task_id(root, &schedule.name);
    let log_directory = log_enabled
        .then(|| schedule_log_directory(root, &schedule.name))
        .transpose()?;
    let stdout = log_directory.as_ref().map(|path| path.join("stdout.log"));
    let stderr = log_directory.as_ref().map(|path| path.join("stderr.log"));
    match platform {
        NativePlatform::Launchd => {
            let destination = home_directory()?
                .join("Library/LaunchAgents")
                .join(format!("{task_id}.plist"));
            let trigger = if let Some(at) = &schedule.at {
                let (hour, minute) = at.split_once(':').expect("validated schedule time");
                format!(
                    "<key>StartCalendarInterval</key><dict><key>Hour</key><integer>{}</integer><key>Minute</key><integer>{}</integer></dict>",
                    hour.parse::<u8>()?,
                    minute.parse::<u8>()?
                )
            } else if let Some(every) = &schedule.every {
                format!(
                    "<key>StartInterval</key><integer>{}</integer>",
                    schedule_interval_seconds(every)?
                )
            } else {
                launchd_cron_trigger(&CronExpression::parse(
                    schedule.cron.as_deref().expect("validated cron"),
                )?)?
            };
            let timezone_environment = timezone.map_or_else(String::new, |timezone| {
                format!(
                    "<key>BATCH_GIT_TZ</key><string>{}</string>",
                    xml_escape(timezone)
                )
            });
            let content = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>ProgramArguments</key><array><string>{}</string><string>schedule</string><string>run</string><string>{}</string></array><key>EnvironmentVariables</key><dict><key>BATCH_GIT_WORKSPACE</key><string>{}</string><key>NO_COLOR</key><string>1</string>{timezone_environment}</dict>{}<key>StandardOutPath</key><string>{}</string><key>StandardErrorPath</key><string>{}</string></dict></plist>\n",
                xml_escape(&task_id),
                xml_escape(&executable.display().to_string()),
                xml_escape(&schedule.name),
                xml_escape(&root.display().to_string()),
                trigger,
                xml_escape(
                    &stdout
                        .as_deref()
                        .unwrap_or(Path::new("/dev/null"))
                        .display()
                        .to_string()
                ),
                xml_escape(
                    &stderr
                        .as_deref()
                        .unwrap_or(Path::new("/dev/null"))
                        .display()
                        .to_string()
                )
            );
            Ok(NativeArtifact {
                platform,
                task_id,
                log_enabled,
                log_directory,
                stdout_path: stdout,
                stderr_path: stderr,
                files: vec![NativeFile {
                    path: destination,
                    content,
                }],
            })
        }
        NativePlatform::Systemd => {
            let directory = systemd_user_directory()?;
            let service_path = directory.join(format!("{task_id}.service"));
            let timer_path = directory.join(format!("{task_id}.timer"));
            let timezone_environment = timezone.map_or_else(
                || "UnsetEnvironment=TZ BATCH_GIT_TZ".to_owned(),
                |timezone| {
                    format!(
                        "Environment={}",
                        systemd_quote(&format!("BATCH_GIT_TZ={timezone}"))
                    )
                },
            );
            let service = format!(
                "[Unit]\nDescription=batch-git schedule {}\n\n[Service]\nType=oneshot\nEnvironment=NO_COLOR=1\nEnvironment={}\n{timezone_environment}\nExecStart={} schedule run {}\nStandardOutput={}\nStandardError={}\n",
                schedule.name,
                systemd_quote(&format!("BATCH_GIT_WORKSPACE={}", root.display())),
                systemd_quote(&executable.display().to_string()),
                systemd_quote(&schedule.name),
                stdout
                    .as_ref()
                    .map(|path| systemd_quote(&format!("append:{}", path.display())))
                    .unwrap_or_else(|| "null".to_owned()),
                stderr
                    .as_ref()
                    .map(|path| systemd_quote(&format!("append:{}", path.display())))
                    .unwrap_or_else(|| "null".to_owned())
            );
            let timezone_suffix =
                timezone.map_or_else(String::new, |timezone| format!(" {timezone}"));
            let timer_trigger = if let Some(at) = &schedule.at {
                format!("OnCalendar=*-*-* {at}:00{timezone_suffix}")
            } else if let Some(every) = &schedule.every {
                format!(
                    "OnUnitActiveSec={}s\nOnBootSec={}s",
                    schedule_interval_seconds(every)?,
                    schedule_interval_seconds(every)?
                )
            } else {
                format!(
                    "{}{timezone_suffix}",
                    systemd_cron_trigger(&CronExpression::parse(
                        schedule.cron.as_deref().expect("validated cron"),
                    )?)
                )
            };
            let timer = format!(
                "[Unit]\nDescription=batch-git schedule {}\n\n[Timer]\n{}\nPersistent=true\nUnit={}.service\n\n[Install]\nWantedBy=timers.target\n",
                schedule.name, timer_trigger, task_id
            );
            Ok(NativeArtifact {
                platform,
                task_id,
                log_enabled,
                log_directory,
                stdout_path: stdout,
                stderr_path: stderr,
                files: vec![
                    NativeFile {
                        path: service_path,
                        content: service,
                    },
                    NativeFile {
                        path: timer_path,
                        content: timer,
                    },
                ],
            })
        }
        NativePlatform::Windows => {
            if schedule.cron.is_some() {
                bail!(
                    "Windows Task Scheduler does not support cron schedules; use --at or --every"
                );
            }
            let destination = windows_task_directory()?.join(format!("{task_id}.xml"));
            let trigger = if let Some(at) = &schedule.at {
                format!(
                    "<CalendarTrigger><StartBoundary>2000-01-01T{at}:00</StartBoundary><Enabled>true</Enabled><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger>"
                )
            } else {
                let seconds = schedule_interval_seconds(
                    schedule
                        .every
                        .as_deref()
                        .expect("validated schedule interval"),
                )?;
                let interval = windows_repetition_interval(seconds)?;
                format!(
                    "<TimeTrigger><Repetition><Interval>{interval}</Interval><StopAtDurationEnd>false</StopAtDurationEnd></Repetition><StartBoundary>2000-01-01T00:00:00</StartBoundary><Enabled>true</Enabled></TimeTrigger>"
                )
            };
            let arguments = if log_enabled || timezone.is_some() {
                let mut arguments = format!("schedule native-run {}", schedule.name);
                if log_enabled {
                    arguments.push_str(" --log");
                }
                if let Some(timezone) = timezone {
                    arguments.push_str(" --timezone ");
                    arguments.push_str(&windows_argument(timezone));
                }
                arguments
            } else {
                format!("schedule run {}", schedule.name)
            };
            let multiple_instances = match schedule.overlap {
                ScheduleOverlap::Skip => "IgnoreNew",
                ScheduleOverlap::Queue => "Queue",
            };
            let content = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Task version=\"1.4\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\"><RegistrationInfo><Description>batch-git schedule {}</Description></RegistrationInfo><Triggers>{trigger}</Triggers><Principals><Principal id=\"Author\"><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>{multiple_instances}</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><AllowHardTerminate>true</AllowHardTerminate><StartWhenAvailable>true</StartWhenAvailable><RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable><Enabled>true</Enabled><Hidden>false</Hidden><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><Priority>7</Priority></Settings><Actions Context=\"Author\"><Exec><Command>{}</Command><Arguments>{}</Arguments><WorkingDirectory>{}</WorkingDirectory></Exec></Actions></Task>\n",
                xml_escape(&schedule.name),
                xml_escape(&executable.display().to_string()),
                xml_escape(&arguments),
                xml_escape(&root.display().to_string()),
            );
            Ok(NativeArtifact {
                platform,
                task_id,
                log_enabled,
                log_directory,
                stdout_path: stdout,
                stderr_path: stderr,
                files: vec![NativeFile {
                    path: destination,
                    content,
                }],
            })
        }
    }
}

fn windows_argument(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character == '"')
    {
        return value.to_owned();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
        } else if character == '"' {
            quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
            quoted.push('"');
            backslashes = 0;
        } else {
            quoted.push_str(&"\\".repeat(backslashes));
            backslashes = 0;
            quoted.push(character);
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

fn windows_repetition_interval(seconds: u64) -> Result<String> {
    const MINIMUM: u64 = 60;
    const MAXIMUM: u64 = 31 * 24 * 60 * 60;
    if seconds < MINIMUM {
        bail!("Windows Task Scheduler requires --every to be at least 1m");
    }
    if seconds > MAXIMUM {
        bail!("Windows Task Scheduler requires --every to be at most 31d");
    }
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3_600;
    let minutes = seconds % 3_600 / 60;
    let seconds = seconds % 60;
    let mut value = String::from("P");
    if days > 0 {
        value.push_str(&format!("{days}D"));
    }
    if hours > 0 || minutes > 0 || seconds > 0 || days == 0 {
        value.push('T');
        if hours > 0 {
            value.push_str(&format!("{hours}H"));
        }
        if minutes > 0 {
            value.push_str(&format!("{minutes}M"));
        }
        if seconds > 0 {
            value.push_str(&format!("{seconds}S"));
        }
    }
    Ok(value)
}

fn launchd_cron_trigger(cron: &CronExpression) -> Result<String> {
    if cron.seconds() != [0] {
        bail!(
            "launchd cron schedules require the second field to be exactly 0; use systemd or an every interval for sub-minute schedules"
        );
    }
    let hours = optional_cron_values(cron.hours(), cron.hours_unrestricted());
    let days = optional_cron_values(cron.days_of_month(), cron.days_of_month_unrestricted());
    let months = optional_cron_values(cron.months(), cron.months_unrestricted());
    let weekdays = optional_cron_values(cron.days_of_week(), cron.days_of_week_unrestricted());
    let count = cron.minutes().len()
        * hours.as_ref().map_or(1, Vec::len)
        * days.as_ref().map_or(1, Vec::len)
        * months.as_ref().map_or(1, Vec::len)
        * weekdays.as_ref().map_or(1, Vec::len);
    if count > 4096 {
        bail!(
            "cron expression expands to {count} launchd calendar entries; maximum supported is 4096"
        );
    }

    let mut dictionaries = Vec::with_capacity(count);
    for minute in cron.minutes() {
        for hour in optional_iter(&hours) {
            for day in optional_iter(&days) {
                for month in optional_iter(&months) {
                    for weekday in optional_iter(&weekdays) {
                        let mut dictionary = String::from("<dict>");
                        push_launchd_integer(&mut dictionary, "Minute", Some(*minute));
                        push_launchd_integer(&mut dictionary, "Hour", hour);
                        push_launchd_integer(&mut dictionary, "Day", day);
                        push_launchd_integer(&mut dictionary, "Month", month);
                        push_launchd_integer(&mut dictionary, "Weekday", weekday);
                        dictionary.push_str("</dict>");
                        dictionaries.push(dictionary);
                    }
                }
            }
        }
    }
    let value = if dictionaries.len() == 1 {
        dictionaries.pop().expect("one launchd calendar entry")
    } else {
        format!("<array>{}</array>", dictionaries.concat())
    };
    Ok(format!("<key>StartCalendarInterval</key>{value}"))
}

fn optional_cron_values(values: &[u8], unrestricted: bool) -> Option<Vec<u8>> {
    (!unrestricted).then(|| values.to_vec())
}

fn optional_iter(values: &Option<Vec<u8>>) -> Vec<Option<u8>> {
    match values {
        Some(values) => values.iter().copied().map(Some).collect(),
        None => vec![None],
    }
}

fn push_launchd_integer(output: &mut String, key: &str, value: Option<u8>) {
    if let Some(value) = value {
        output.push_str(&format!("<key>{key}</key><integer>{value}</integer>"));
    }
}

fn systemd_cron_trigger(cron: &CronExpression) -> String {
    let weekday = if cron.days_of_week_unrestricted() {
        String::new()
    } else {
        format!(
            "{} ",
            cron.days_of_week()
                .iter()
                .map(|value| match value {
                    0 => "Sun",
                    1 => "Mon",
                    2 => "Tue",
                    3 => "Wed",
                    4 => "Thu",
                    5 => "Fri",
                    6 => "Sat",
                    _ => unreachable!("validated weekday"),
                })
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let month = cron_component(cron.months(), cron.months_unrestricted());
    let day = cron_component(cron.days_of_month(), cron.days_of_month_unrestricted());
    format!(
        "OnCalendar={weekday}*-{month}-{day} {}:{}:{}",
        cron_component(cron.hours(), cron.hours_unrestricted()),
        cron_component(cron.minutes(), cron.minutes_unrestricted()),
        cron_component(cron.seconds(), cron.seconds_unrestricted())
    )
}

fn cron_component(values: &[u8], unrestricted: bool) -> String {
    if unrestricted {
        "*".to_owned()
    } else {
        join_numbers(values)
    }
}

fn join_numbers(values: &[u8]) -> String {
    values
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn print_artifact(artifact: &NativeArtifact) {
    for (index, file) in artifact.files.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("# {}", file.path.display());
        print!("{}", file.content);
    }
}

fn write_and_activate(artifact: &NativeArtifact, updating: bool) -> Result<()> {
    if let Some(log_directory) = &artifact.log_directory {
        fs::create_dir_all(log_directory).with_context(|| {
            format!(
                "failed to create schedule log directory {}",
                log_directory.display()
            )
        })?;
    }
    let backups = artifact
        .files
        .iter()
        .map(|file| fs::read(&file.path).ok())
        .collect::<Vec<_>>();
    for file in &artifact.files {
        atomic_write(&file.path, file.content.as_bytes())?;
    }
    if let Err(error) = activate(artifact, updating) {
        for (file, backup) in artifact.files.iter().zip(backups) {
            match backup {
                Some(content) => {
                    let _ = atomic_write(&file.path, &content);
                }
                None => {
                    let _ = fs::remove_file(&file.path);
                }
            }
        }
        let _ = reactivate_restored_files(artifact);
        return Err(error);
    }
    Ok(())
}

fn activate(artifact: &NativeArtifact, updating: bool) -> Result<()> {
    match artifact.platform {
        NativePlatform::Launchd => {
            let domain = launchd_domain()?;
            if updating {
                let target = format!("{domain}/{}", artifact.task_id);
                let _ = Command::new("launchctl")
                    .args(["bootout", &target])
                    .status();
            }
            let status = Command::new("launchctl")
                .args(["bootstrap", &domain])
                .arg(&artifact.files[0].path)
                .status()
                .context("failed to execute launchctl")?;
            if !status.success() {
                bail!("launchctl bootstrap failed with {status}");
            }
        }
        NativePlatform::Systemd => {
            run_systemctl(["daemon-reload"])?;
            let timer = format!("{}.timer", artifact.task_id);
            if updating {
                run_systemctl(["restart", timer.as_str()])?;
                run_systemctl(["enable", timer.as_str()])?;
            } else {
                run_systemctl(["enable", "--now", timer.as_str()])?;
            }
        }
        NativePlatform::Windows => {
            let status = Command::new("schtasks.exe")
                .args(["/Create", "/TN", &artifact.task_id, "/XML"])
                .arg(&artifact.files[0].path)
                .arg("/F")
                .status()
                .context("failed to execute schtasks.exe")?;
            if !status.success() {
                bail!("schtasks.exe /Create failed with {status}");
            }
        }
    }
    Ok(())
}

fn deactivate(state: &RegistrationState) -> Result<()> {
    match state.platform.as_str() {
        "launchd" => {
            let domain = launchd_domain()?;
            let target = format!("{domain}/{}", state.task_id);
            let _ = Command::new("launchctl")
                .args(["bootout", &target])
                .status();
        }
        "systemd" => {
            let timer = format!("{}.timer", state.task_id);
            let _ = Command::new("systemctl")
                .args(["--user", "disable", "--now", timer.as_str()])
                .status();
        }
        "windows" => {
            let _ = Command::new("schtasks.exe")
                .args(["/Delete", "/TN", &state.task_id, "/F"])
                .status();
        }
        platform => bail!("unsupported registered schedule platform: {platform}"),
    }
    Ok(())
}

fn reactivate_restored_files(artifact: &NativeArtifact) -> Result<()> {
    match artifact.platform {
        NativePlatform::Launchd => {
            if artifact.files[0].path.is_file() {
                let domain = launchd_domain()?;
                let status = Command::new("launchctl")
                    .args(["bootstrap", &domain])
                    .arg(&artifact.files[0].path)
                    .status()
                    .context("failed to restore previous launchd task")?;
                if !status.success() {
                    bail!("failed to restore previous launchd task: {status}");
                }
            }
        }
        NativePlatform::Systemd => {
            run_systemctl(["daemon-reload"])?;
            let timer = format!("{}.timer", artifact.task_id);
            let _ = run_systemctl(["restart", timer.as_str()]);
        }
        NativePlatform::Windows => {
            if artifact.files[0].path.is_file() {
                let status = Command::new("schtasks.exe")
                    .args(["/Create", "/TN", &artifact.task_id, "/XML"])
                    .arg(&artifact.files[0].path)
                    .arg("/F")
                    .status()
                    .context("failed to restore previous Windows scheduled task")?;
                if !status.success() {
                    bail!("failed to restore previous Windows scheduled task: {status}");
                }
            }
        }
    }
    Ok(())
}

fn run_systemctl<const N: usize>(arguments: [&str; N]) -> Result<()> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(arguments)
        .status()
        .context("failed to execute systemctl")?;
    if !status.success() {
        bail!("systemctl failed with {status}");
    }
    Ok(())
}

fn native_task_exists(platform: NativePlatform, task_id: &str) -> Result<bool> {
    if platform != NativePlatform::Windows {
        return Ok(false);
    }
    #[cfg(target_os = "windows")]
    {
        let status = Command::new("schtasks.exe")
            .args(["/Query", "/TN", task_id])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("failed to execute schtasks.exe")?;
        Ok(status.success())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = task_id;
        Ok(false)
    }
}

fn launchd_domain() -> Result<String> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("failed to determine user id")?;
    if !output.status.success() {
        bail!("id -u failed with {}", output.status);
    }
    Ok(format!("gui/{}", String::from_utf8(output.stdout)?.trim()))
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    temporary.write_all(content)?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .ok();
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct RegistrationState {
    workspace: String,
    name: String,
    platform: String,
    task_id: String,
    digest: String,
    files: Vec<String>,
    #[serde(default = "default_true")]
    log_enabled: bool,
    #[serde(default)]
    stdout_path: Option<String>,
    #[serde(default)]
    stderr_path: Option<String>,
    registered_at: String,
    updated_at: String,
}

fn default_true() -> bool {
    true
}

fn yes_no(value: bool) -> String {
    if value { "yes" } else { "no" }.to_owned()
}

struct NativeStatus {
    loaded: bool,
    state: Option<String>,
    runs: Option<u64>,
    last_exit_code: Option<i32>,
}

fn native_status(state: &RegistrationState) -> Result<NativeStatus> {
    match state.platform.as_str() {
        "launchd" => {
            let target = format!("{}/{}", launchd_domain()?, state.task_id);
            let output = Command::new("launchctl")
                .args(["print", &target])
                .output()
                .context("failed to execute launchctl")?;
            let text = String::from_utf8_lossy(&output.stdout);
            Ok(NativeStatus {
                loaded: output.status.success(),
                state: parse_native_value(&text, "state"),
                runs: parse_native_value(&text, "runs").and_then(|value| value.parse().ok()),
                last_exit_code: parse_native_value(&text, "last exit code")
                    .and_then(|value| value.parse().ok()),
            })
        }
        "systemd" => {
            let timer = format!("{}.timer", state.task_id);
            let timer_output = Command::new("systemctl")
                .args([
                    "--user",
                    "show",
                    &timer,
                    "--property=LoadState",
                    "--property=ActiveState",
                    "--property=SubState",
                ])
                .output()
                .context("failed to execute systemctl")?;
            let timer_text = String::from_utf8_lossy(&timer_output.stdout);
            let service = format!("{}.service", state.task_id);
            let service_output = Command::new("systemctl")
                .args([
                    "--user",
                    "show",
                    &service,
                    "--property=Result",
                    "--property=ExecMainStatus",
                    "--property=InvocationID",
                ])
                .output()
                .context("failed to execute systemctl")?;
            let service_text = String::from_utf8_lossy(&service_output.stdout);
            let active = parse_systemd_value(&timer_text, "ActiveState");
            let sub = parse_systemd_value(&timer_text, "SubState");
            Ok(NativeStatus {
                loaded: timer_output.status.success()
                    && parse_systemd_value(&timer_text, "LoadState").as_deref() == Some("loaded"),
                state: match (active, sub) {
                    (Some(active), Some(sub)) => Some(format!("{active}/{sub}")),
                    (active, sub) => active.or(sub),
                },
                runs: None,
                last_exit_code: if service_output.status.success()
                    && parse_systemd_value(&service_text, "InvocationID")
                        .is_some_and(|value| !value.is_empty())
                {
                    parse_systemd_value(&service_text, "ExecMainStatus")
                        .and_then(|value| value.parse().ok())
                } else {
                    None
                },
            })
        }
        "windows" => {
            let output = Command::new("schtasks.exe")
                .args(["/Query", "/TN", &state.task_id])
                .output()
                .context("failed to execute schtasks.exe")?;
            Ok(NativeStatus {
                loaded: output.status.success(),
                state: None,
                runs: None,
                last_exit_code: None,
            })
        }
        platform => bail!("unsupported registered schedule platform: {platform}"),
    }
}

fn parse_native_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (name, value) = line.trim().split_once('=')?;
        (name.trim() == key).then(|| value.trim().to_owned())
    })
}

fn parse_systemd_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name == key).then(|| value.to_owned())
    })
}

fn load_state(root: &Path, name: &str) -> Result<Option<RegistrationState>> {
    let path = registration_state_path(root, name)?;
    if !path.is_file() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut state: RegistrationState = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if state.log_enabled && (state.stdout_path.is_none() || state.stderr_path.is_none()) {
        let directory = schedule_log_directory(root, name)?;
        state.stdout_path = Some(directory.join("stdout.log").display().to_string());
        state.stderr_path = Some(directory.join("stderr.log").display().to_string());
    }
    validate_state(root, name, &state)?;
    Ok(Some(state))
}

fn write_state(root: &Path, state: &RegistrationState) -> Result<()> {
    let path = registration_state_path(root, &state.name)?;
    atomic_write(&path, serde_json::to_string_pretty(state)?.as_bytes())
}

fn registration_state_path(root: &Path, name: &str) -> Result<PathBuf> {
    crate::model::validate_schedule_name(name)?;
    Ok(state_root()?
        .join("registrations")
        .join(format!(
            "{:016x}",
            fnv1a(root.display().to_string().as_bytes())
        ))
        .join(format!("{name}.json")))
}

fn schedule_log_directory(root: &Path, name: &str) -> Result<PathBuf> {
    crate::model::validate_schedule_name(name)?;
    Ok(state_root()?
        .join("logs")
        .join(format!(
            "{:016x}",
            fnv1a(root.display().to_string().as_bytes())
        ))
        .join(name))
}

fn state_root() -> Result<PathBuf> {
    if let Some(value) = env::var_os("BATCH_GIT_STATE_DIR") {
        return Ok(PathBuf::from(value));
    }
    #[cfg(target_os = "macos")]
    {
        Ok(home_directory()?.join("Library/Application Support/batch-git"))
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(value) = env::var_os("LOCALAPPDATA").or_else(|| env::var_os("APPDATA")) {
            return Ok(PathBuf::from(value).join("batch-git"));
        }
        Ok(home_directory()?.join("AppData/Local/batch-git"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if let Some(value) = env::var_os("XDG_STATE_HOME") {
            return Ok(PathBuf::from(value).join("batch-git"));
        }
        Ok(home_directory()?.join(".local/state/batch-git"))
    }
}

fn systemd_user_directory() -> Result<PathBuf> {
    if let Some(value) = env::var_os("XDG_CONFIG_HOME") {
        Ok(PathBuf::from(value).join("systemd/user"))
    } else {
        Ok(home_directory()?.join(".config/systemd/user"))
    }
}

fn windows_task_directory() -> Result<PathBuf> {
    Ok(state_root()?.join("tasks/windows"))
}

fn home_directory() -> Result<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME and USERPROFILE are not set"))
}

fn remove_registered_files(state: &RegistrationState) -> Result<()> {
    for file in &state.files {
        let path = PathBuf::from(file);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
    }
    if state.platform == "systemd" {
        run_systemctl(["daemon-reload"])?;
    }
    Ok(())
}

fn validate_state(root: &Path, name: &str, state: &RegistrationState) -> Result<()> {
    if state.workspace != root.display().to_string() || state.name != name {
        bail!("schedule registration state does not match this workspace and schedule");
    }
    let platform = NativePlatform::from_label(&state.platform)?;
    if state.task_id != task_id(root, name) {
        bail!("schedule registration contains an invalid task id");
    }
    let expected = native_paths(&state.task_id, platform)?;
    let actual = state.files.iter().map(PathBuf::from).collect::<Vec<_>>();
    if actual != expected {
        bail!("schedule registration contains unexpected native task paths");
    }
    Ok(())
}

fn native_paths(task_id: &str, platform: NativePlatform) -> Result<Vec<PathBuf>> {
    match platform {
        NativePlatform::Launchd => Ok(vec![
            home_directory()?
                .join("Library/LaunchAgents")
                .join(format!("{task_id}.plist")),
        ]),
        NativePlatform::Systemd => {
            let directory = systemd_user_directory()?;
            Ok(vec![
                directory.join(format!("{task_id}.service")),
                directory.join(format!("{task_id}.timer")),
            ])
        }
        NativePlatform::Windows => Ok(vec![
            windows_task_directory()?.join(format!("{task_id}.xml")),
        ]),
    }
}

fn artifact_files_match(artifact: &NativeArtifact) -> Result<bool> {
    for file in &artifact.files {
        match fs::read_to_string(&file.path) {
            Ok(content) if content == file.content => {}
            Ok(_) | Err(_) => return Ok(false),
        }
    }
    Ok(true)
}

fn artifact_digest(artifact: &NativeArtifact) -> String {
    let mut content = artifact.platform.label().to_owned();
    content.push_str(&artifact.task_id);
    for file in &artifact.files {
        content.push_str(&file.path.display().to_string());
        content.push_str(&file.content);
    }
    format!("{:016x}", fnv1a(content.as_bytes()))
}

fn task_id(root: &Path, name: &str) -> String {
    format!(
        "com.batch-git.{:016x}.{}",
        fnv1a(root.display().to_string().as_bytes()),
        name.replace('_', "-")
    )
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn systemd_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::{
        NativePlatform, artifact_digest, build_artifact, build_artifact_with_logging,
        build_artifact_with_options, fnv1a, launchd_cron_trigger, systemd_cron_trigger,
        systemd_quote, windows_argument, windows_repetition_interval, xml_escape,
    };
    use crate::cron::CronExpression;
    use crate::model::{ScheduleAction, ScheduleOverlap, ScheduleRecord, ScheduleScope};

    #[test]
    fn generated_identifiers_and_escaping_are_stable() {
        assert_eq!(fnv1a(b"workspace"), 0x40e26138f4336c36);
        assert_eq!(xml_escape("a&<b>"), "a&amp;&lt;b&gt;");
        assert_eq!(systemd_quote("a b"), "\"a b\"");
    }

    #[test]
    fn same_name_keeps_its_task_id_while_configuration_changes_digest() {
        let root = tempfile::tempdir().unwrap();
        let schedule = |at: &str| ScheduleRecord {
            name: "nightly-sync".to_owned(),
            enabled: true,
            action: ScheduleAction::Sync,
            at: Some(at.to_owned()),
            every: None,
            cron: None,
            timezone: "local".to_owned(),
            overlap: ScheduleOverlap::Skip,
            scope: ScheduleScope {
                all: true,
                repositories: Vec::new(),
            },
        };
        let first =
            build_artifact(root.path(), &schedule("02:30"), NativePlatform::Launchd).unwrap();
        let updated =
            build_artifact(root.path(), &schedule("03:30"), NativePlatform::Launchd).unwrap();
        assert_eq!(first.task_id, updated.task_id);
        assert_ne!(artifact_digest(&first), artifact_digest(&updated));
    }

    #[test]
    fn cron_generates_native_calendar_definitions() {
        let cron = CronExpression::parse("0 */15 9-17 * * MON-FRI").unwrap();
        let launchd = launchd_cron_trigger(&cron).unwrap();
        assert!(launchd.starts_with("<key>StartCalendarInterval</key><array>"));
        assert!(launchd.contains("<key>Minute</key><integer>15</integer>"));
        assert!(launchd.contains("<key>Hour</key><integer>9</integer>"));
        assert!(launchd.contains("<key>Weekday</key><integer>5</integer>"));
        assert_eq!(
            systemd_cron_trigger(&cron),
            "OnCalendar=Mon,Tue,Wed,Thu,Fri *-*-* 9,10,11,12,13,14,15,16,17:0,15,30,45:0"
        );

        let with_seconds = CronExpression::parse("*/10 * * * * *").unwrap();
        assert!(launchd_cron_trigger(&with_seconds).is_err());
        assert_eq!(
            systemd_cron_trigger(&with_seconds),
            "OnCalendar=*-*-* *:*:0,10,20,30,40,50"
        );
    }

    #[test]
    fn windows_generates_daily_and_interval_tasks_but_rejects_cron() {
        let root = tempfile::tempdir().unwrap();
        let schedule =
            |at: Option<&str>, every: Option<&str>, cron: Option<&str>, overlap| ScheduleRecord {
                name: "windows-sync".to_owned(),
                enabled: true,
                action: ScheduleAction::Sync,
                at: at.map(str::to_owned),
                every: every.map(str::to_owned),
                cron: cron.map(str::to_owned),
                timezone: "local".to_owned(),
                overlap,
                scope: ScheduleScope {
                    all: true,
                    repositories: Vec::new(),
                },
            };

        let daily = build_artifact(
            root.path(),
            &schedule(Some("02:30"), None, None, ScheduleOverlap::Skip),
            NativePlatform::Windows,
        )
        .unwrap();
        assert_eq!(daily.files.len(), 1);
        assert_eq!(daily.files[0].path.extension().unwrap(), "xml");
        assert!(
            daily.files[0]
                .content
                .contains("<StartBoundary>2000-01-01T02:30:00</StartBoundary>")
        );
        assert!(
            daily.files[0]
                .content
                .contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>")
        );
        assert!(
            daily.files[0]
                .content
                .contains("<Arguments>schedule run windows-sync</Arguments>")
        );

        let interval = build_artifact_with_logging(
            root.path(),
            &schedule(None, Some("15m"), None, ScheduleOverlap::Queue),
            NativePlatform::Windows,
            true,
        )
        .unwrap();
        assert!(
            interval.files[0]
                .content
                .contains("<Interval>PT15M</Interval>")
        );
        assert!(
            interval.files[0]
                .content
                .contains("<MultipleInstancesPolicy>Queue</MultipleInstancesPolicy>")
        );
        assert!(
            interval.files[0]
                .content
                .contains("<Arguments>schedule native-run windows-sync --log</Arguments>")
        );

        let timezone = build_artifact_with_options(
            root.path(),
            &schedule(Some("02:30"), None, None, ScheduleOverlap::Skip),
            NativePlatform::Windows,
            false,
            Some("Asia/Shanghai"),
        )
        .unwrap();
        assert!(timezone.files[0].content.contains(
            "<Arguments>schedule native-run windows-sync --timezone Asia/Shanghai</Arguments>"
        ));

        let cron = build_artifact(
            root.path(),
            &schedule(None, None, Some("0 0 2 * * *"), ScheduleOverlap::Skip),
            NativePlatform::Windows,
        )
        .err()
        .expect("Windows cron should be rejected");
        assert!(cron.to_string().contains("does not support cron schedules"));
        assert!(windows_repetition_interval(30).is_err());
        assert_eq!(windows_repetition_interval(90).unwrap(), "PT1M30S");
        assert_eq!(windows_repetition_interval(86_400).unwrap(), "P1D");
        assert!(windows_repetition_interval(32 * 86_400).is_err());
        assert_eq!(windows_argument("Asia/Shanghai"), "Asia/Shanghai");
        assert_eq!(windows_argument("value with space"), "\"value with space\"");
    }
}
