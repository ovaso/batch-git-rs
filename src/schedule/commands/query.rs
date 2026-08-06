//! Read-only schedule list, status, and validation commands.

use super::execution::build_plan;
use super::support::{action_label, find_schedule, trigger_label};
use super::*;

#[derive(Serialize)]
struct ScheduleListItem {
    name: String,
    enabled: bool,
    action: &'static str,
    trigger: String,
    scope: String,
    registered: Option<String>,
}

/// List declarations and optionally their locally registered platform.
pub(super) fn list(arguments: ScheduleListArgs, context: &CommandContext<'_>) -> Result<i32> {
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
    if context.automation.is_machine() {
        context.emit_data("list", &root, 0, &items)?;
    } else if arguments.json {
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

/// Compare the declaration, registration state, native files, and live scheduler status.
pub(super) fn status(arguments: ScheduleNameArgs, context: &CommandContext<'_>) -> Result<i32> {
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
    if context.automation.is_machine() {
        context.emit_data("status", &root, 0, &output)?;
    } else if arguments.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print_status(output);
    }
    Ok(0)
}

fn print_status(output: StatusOutput) {
    let rows = vec![
        vec!["NAME".to_owned(), output.name],
        vec!["DECLARED".to_owned(), yes_no(output.declared)],
        vec![
            "ENABLED".to_owned(),
            output.enabled.map(yes_no).unwrap_or_else(|| "-".to_owned()),
        ],
        vec![
            "TRIGGER".to_owned(),
            output.trigger.unwrap_or_else(|| "-".to_owned()),
        ],
        vec!["REGISTERED".to_owned(), yes_no(output.registered)],
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

/// Validate that declarations can be mapped to a target native scheduler without writing files.
pub(super) fn doctor(arguments: ScheduleDoctorArgs, context: &CommandContext<'_>) -> Result<i32> {
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
            "repositories": plan.repository_count(),
            "task_id": artifact.task_id,
            "status": "ok"
        }));
    }
    if context.automation.is_machine() {
        context.emit_data("doctor", &root, 0, &checks)?;
    } else if arguments.json {
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
