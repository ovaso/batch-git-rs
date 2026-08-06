//! Schedule planning and execution through the regular command workflows.

use super::support::{
    action_label, find_schedule, overlap_label, selected_repositories, trigger_label,
};
use super::*;

/// Shared model for human and machine `schedule plan` output.
#[derive(Debug, Serialize)]
pub(super) struct PlanOutput {
    schedule: String,
    action: &'static str,
    trigger: String,
    overlap: &'static str,
    repositories: Vec<String>,
}

impl PlanOutput {
    pub(super) fn repository_count(&self) -> usize {
        self.repositories.len()
    }
}

/// Show the resolved action and repository scope without side effects.
pub(super) fn plan(arguments: ScheduleNameArgs, context: &CommandContext<'_>) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let output = build_plan(&manifest, &arguments.name)?;
    if context.automation.is_machine() {
        context.emit_data("plan", &root, 0, &output)?;
    } else if arguments.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print_plan(&output);
    }
    Ok(0)
}

/// Build a validated, fully resolved schedule execution plan.
pub(super) fn build_plan(manifest: &Workspace, name: &str) -> Result<PlanOutput> {
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

/// Execute a declaration while honoring its overlap policy.
pub(super) fn run(arguments: ScheduleRunArgs, context: &CommandContext<'_>) -> Result<i32> {
    let root = workspace::find_root()?;
    let initial = workspace::read(&root)?;
    let overlap = find_schedule(&initial, &arguments.name)?.overlap;
    let _lock = match overlap {
        ScheduleOverlap::Queue => WorkspaceLock::acquire(&root)?,
        ScheduleOverlap::Skip => match WorkspaceLock::try_acquire(&root)? {
            Some(lock) => lock,
            None => {
                if context.automation.is_machine() {
                    context.emit_data(
                        "run",
                        &root,
                        0,
                        &json!({
                            "schedule": arguments.name,
                            "status": "skipped",
                            "reason_code": "workspace_locked",
                        }),
                    )?;
                } else {
                    println!("schedule {} skipped: workspace is locked", arguments.name);
                }
                return Ok(0);
            }
        },
    };
    context.verify_apply_revision(&root)?;
    let mut manifest = workspace::read(&root)?;
    let schedule = find_schedule(&manifest, &arguments.name)?.clone();
    if !schedule.enabled {
        bail!("schedule is disabled: {}", schedule.name);
    }
    let repositories = selected_repositories(&manifest, &schedule)?;
    match schedule.action {
        ScheduleAction::Sync => crate::commands::run_sync_named(
            &root,
            &mut manifest,
            &repositories,
            context.jobs,
            context.verbose,
            context.automation,
            "schedule run",
        ),
        ScheduleAction::Pull => crate::commands::run_pull_named(
            &root,
            &mut manifest,
            &repositories,
            context.jobs,
            context.verbose,
            context.automation,
            "schedule run",
        ),
    }
}

/// Build the invocation forwarded by a native scheduler to the regular schedule runner.
///
/// Native schedulers are always unattended. Force the child to be non-interactive even when the
/// outer `native-run` invocation uses text output, so Git cannot block on a terminal prompt.
pub(in crate::schedule) fn native_run_child_arguments(
    name: &str,
    jobs: usize,
    automation: &AutomationOptions,
) -> Vec<String> {
    let mut child_arguments = Vec::new();
    child_arguments.push("--jobs".to_owned());
    child_arguments.push(jobs.to_string());
    child_arguments.push("--non-interactive".to_owned());
    if let Some(timeout) = automation.timeout {
        child_arguments.push("--timeout".to_owned());
        child_arguments.push(format!("{}s", timeout.as_secs()));
    }
    if automation.apply {
        child_arguments.push("--apply".to_owned());
        child_arguments.push("--expect-workspace-revision".to_owned());
        child_arguments.push(
            automation
                .expected_workspace_revision
                .clone()
                .expect("validated --apply must include a workspace revision"),
        );
    }
    child_arguments.extend(["schedule".to_owned(), "run".to_owned(), name.to_owned()]);
    child_arguments
}

/// Hidden native scheduler entry point with stable environment and log redirection.
pub(super) fn native_run(
    arguments: ScheduleNativeRunArgs,
    context: &CommandContext<'_>,
) -> Result<i32> {
    let root = workspace::find_root()?;
    // The child acquires the workspace lock before repeating the revision check and execution.
    context.verify_apply_revision(&root)?;
    let child_arguments =
        native_run_child_arguments(&arguments.name, context.jobs, context.automation);
    let mut command =
        Command::new(env::current_exe().context("failed to locate batch-git executable")?);
    command
        .args(&child_arguments)
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
    let exit_code = status.code().unwrap_or(1);
    if exit_code == 2 {
        // Preserve a stable stale-plan error when the child rejects the second revision check.
        context.verify_apply_revision(&root)?;
        bail!(
            "scheduled batch-git run could not start safely (child exited with status {exit_code})"
        );
    }
    if context.automation.is_machine() {
        context.emit_data(
            "native-run",
            &root,
            exit_code,
            &json!({
                "schedule": arguments.name,
                "status": if exit_code == 0 { "completed" } else { "failed" },
                "child_exit_code": exit_code,
            }),
        )?;
    }
    Ok(exit_code)
}
