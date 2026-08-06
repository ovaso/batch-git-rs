//! Built-in command orchestration.
#![deny(clippy::wildcard_imports)]

mod automation_commands;
mod branches;
mod changes;
mod exec;
mod inspect;
mod plan;
mod remote;
mod workspace_commands;

pub use exec::passthrough;
#[cfg(feature = "schedule")]
pub(crate) use remote::{run_pull_named, run_sync_named};

use plan::plan_passthrough;
use workspace_commands::{default_clone_directory, relative_string, restore_one};

use std::io::{self, IsTerminal};
use std::path::Path;

use anyhow::{Result, bail};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::automation::AutomationOptions;
use crate::cli::{CheckoutArgs, Cli, Command, ExecArgs, MergeArgs};
use crate::git::GitExecutionOptions;
use crate::model::{RepositoryRecord, Workspace};
use crate::parallel::map_ordered_with_completion;
use crate::report::{JsonlProgress, RepositoryResult};
use crate::settings;
use crate::workspace;

/// 解析公共运行配置，并把顶层子命令分派到对应工作流。
pub fn dispatch(cli: Cli) -> Result<i32> {
    let runtime = cli.runtime_options()?;
    let jobs = settings::jobs(runtime.jobs)?;
    let automation = runtime.automation();
    if let Command::Commit(arguments) = &cli.command {
        changes::validate_commit_message(&arguments.message)?;
    }
    if automation.plan {
        return plan::plan_command(&cli.command, jobs, &automation);
    }
    if automation.apply && !cli.command.is_mutating() {
        bail!("--apply is only valid for an operation with side effects");
    }
    match cli.command {
        Command::Add(arguments) => changes::add(arguments, jobs, runtime.verbose, &automation),
        Command::Clone(arguments) => workspace_commands::clone_repository(arguments, &automation),
        Command::Commit(arguments) => {
            changes::commit(arguments, jobs, runtime.verbose, &automation)
        }
        Command::Env(arguments) => automation_commands::environment(arguments, jobs, &automation),
        Command::Scan(arguments) => workspace_commands::scan(arguments, jobs, &automation),
        Command::Restore => workspace_commands::restore(jobs, runtime.verbose, &automation),
        Command::Fetch => remote::fetch(jobs, runtime.verbose, &automation),
        Command::Sync(arguments) => remote::sync(arguments, jobs, runtime.verbose, &automation),
        #[cfg(feature = "schedule")]
        Command::Schedule(arguments) => {
            crate::schedule::dispatch(arguments, jobs, runtime.verbose, &automation)
        }
        Command::Checkout(arguments) => {
            branches::checkout(arguments, jobs, runtime.verbose, &automation)
        }
        Command::Cd => branches::checkout(
            checkout_alias_arguments(true, false),
            jobs,
            runtime.verbose,
            &automation,
        ),
        Command::Cf => branches::checkout(
            checkout_alias_arguments(false, true),
            jobs,
            runtime.verbose,
            &automation,
        ),
        Command::Merge(arguments) => branches::merge(arguments, jobs, runtime.verbose, &automation),
        Command::Pull(arguments) => remote::pull(arguments, jobs, runtime.verbose, &automation),
        Command::Push(arguments) => remote::push(arguments, jobs, runtime.verbose, &automation),
        Command::Exec(arguments) => exec::exec(arguments, jobs, runtime.verbose, &automation),
        Command::Status(arguments) => inspect::status(arguments, jobs, &automation),
        Command::Find(arguments) => inspect::find(arguments, jobs, &automation),
        Command::Info(arguments) => inspect::info(arguments, jobs, &automation),
        Command::Branch(arguments) => inspect::branch(arguments, jobs, &automation),
        Command::Capabilities => automation_commands::capabilities(&automation),
        Command::Schema(arguments) => automation_commands::schema(arguments, &automation),
        Command::List(arguments) => inspect::list(arguments, jobs, &automation),
        Command::Forget(arguments) => workspace_commands::forget(arguments, &automation),
        Command::Unstage(arguments) => {
            changes::unstage(arguments, jobs, runtime.verbose, &automation)
        }
    }
}

fn merge_update_current_setting(arguments: &MergeArgs) -> Option<bool> {
    if arguments.update_current {
        Some(true)
    } else if arguments.no_update_current {
        Some(false)
    } else {
        None
    }
}

fn merge_refresh_source_setting(arguments: &MergeArgs) -> Option<bool> {
    if arguments.refresh_source {
        Some(true)
    } else if arguments.no_refresh_source {
        Some(false)
    } else {
        None
    }
}

struct MergeSettings {
    update_current: bool,
    refresh_source: bool,
}

/// Resolve source-specific defaults first, then let explicit CLI flags override them.
fn merge_settings(arguments: &MergeArgs) -> Result<MergeSettings> {
    let update_current = match merge_update_current_setting(arguments) {
        Some(value) => value,
        None if arguments.feature => settings::merge_feature_update_current()?,
        None => false,
    };
    let refresh_source = match merge_refresh_source_setting(arguments) {
        Some(value) => value,
        None if arguments.default => settings::merge_default_refresh_source()?,
        None => false,
    };
    Ok(MergeSettings {
        update_current,
        refresh_source,
    })
}

/// Enforce the manifest precondition carried from a preceding plan before a write begins.
fn verify_apply_revision(root: &Path, automation: &AutomationOptions) -> Result<()> {
    if let Some(expected) = automation.expected_workspace_revision.as_deref() {
        workspace::verify_revision(root, expected)?;
    }
    Ok(())
}

/// Resolve the `exec` subset once, preserving manifest order and its precise selector errors.
fn select_exec_repositories(
    manifest: &Workspace,
    arguments: &ExecArgs,
) -> Result<Vec<RepositoryRecord>> {
    crate::selector::select(manifest, &arguments.selectors, &arguments.matches, false)
}

/// Derive one child-process policy for an invocation. Machine output is necessarily
/// non-interactive: inheriting a terminal child would violate the JSON stdout contract.
fn git_execution_options(automation: &AutomationOptions, allow_stdin: bool) -> GitExecutionOptions {
    let non_interactive = automation.non_interactive || automation.is_machine();
    GitExecutionOptions {
        allow_stdin: allow_stdin && !non_interactive,
        non_interactive,
        timeout: automation.timeout,
    }
}

/// Execute repository work with JSONL lifecycle events and a stable result vector.
///
/// `JsonlProgress` emits `started` before workers begin, then preserves the protocol's manifest
/// ordering by buffering out-of-order worker completions until their predecessors are available.
fn map_repository_results<T, F>(
    items: &[T],
    jobs: usize,
    automation: &AutomationOptions,
    command: &str,
    root: &Path,
    operation: F,
) -> Result<Vec<RepositoryResult>>
where
    T: Sync,
    F: Fn(&T) -> RepositoryResult + Sync + Send,
{
    let progress = JsonlProgress::new(automation, command, root, items.len())?;
    map_ordered_with_completion(items, jobs, operation, |index, result| {
        if let Some(progress) = &progress {
            progress.repository_finished(index, result);
        }
    })
}

/// 将 `cd`、`cf` 两个便捷别名转换为标准 checkout 参数。
fn checkout_alias_arguments(default: bool, feature: bool) -> CheckoutArgs {
    CheckoutArgs {
        create: false,
        branch: None,
        default,
        feature,
        from: None,
        remote: None,
    }
}

/// clone/restore/fetch 共用的多仓库终端进度状态。
struct OperationProgress {
    multi: Option<MultiProgress>,
    bars: Vec<(String, ProgressBar)>,
}

impl OperationProgress {
    /// 仅在交互终端创建进度条；管道和 CI 中保持稳定表格输出。
    fn new(repositories: &[RepositoryRecord], machine: bool) -> Self {
        if machine || !io::stderr().is_terminal() {
            return Self {
                multi: None,
                bars: Vec::new(),
            };
        }
        let multi = MultiProgress::new();
        let name_width = repositories
            .iter()
            .map(|repository| repository.name.chars().count())
            .max()
            .unwrap_or(10)
            .max("REPOSITORY".len());
        let style = ProgressStyle::with_template(&format!(
            "{{prefix:<{name_width}}}  [{{bar:28.cyan/blue}}] {{pos:>3}}%  {{msg}}"
        ))
        .expect("valid operation progress template")
        .progress_chars("━━╸");
        let bars = repositories
            .iter()
            .map(|repository| {
                let bar = multi.add(ProgressBar::new(100));
                bar.set_style(style.clone());
                bar.set_prefix(repository.name.clone());
                bar.set_message("queued");
                (repository.name.clone(), bar)
            })
            .collect();
        Self {
            multi: Some(multi),
            bars,
        }
    }

    fn bar(&self, repository: &RepositoryRecord) -> Option<ProgressBar> {
        self.bars
            .iter()
            .find(|(name, _)| name == &repository.name)
            .map(|(_, bar)| bar.clone())
    }

    fn finish(self) {
        if let Some(multi) = self.multi {
            let _ = multi.clear();
        }
    }
}

/// 更新可选进度条的阶段文本和位置。
fn set_operation_status(progress: Option<&ProgressBar>, message: &'static str, position: u64) {
    if let Some(progress) = progress {
        progress.set_position(position);
        progress.set_message(message);
    }
}

/// 用仓库最终状态结束进度条。
fn finish_operation(progress: Option<&ProgressBar>, result: &RepositoryResult) {
    if let Some(progress) = progress {
        progress.set_position(100);
        progress.finish_with_message(result.progress_label());
    }
}
