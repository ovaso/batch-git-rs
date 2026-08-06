//! Explicit Git passthrough and selected repository execution.

use std::ffi::OsString;

use anyhow::Result;

use super::{
    git_execution_options, map_repository_results, plan_passthrough, select_exec_repositories,
    verify_apply_revision,
};
use crate::automation::AutomationOptions;
use crate::cli::{ExecArgs, RuntimeOptions};
use crate::git::{self, GitOutput};
use crate::model::RepositoryRecord;
use crate::report::{RepositoryResult, print_results, print_selected_results};
use crate::settings;
use crate::workspace::{self, WorkspaceLock};

/// 在所有已物化仓库中原样执行 `--` 后的 Git 参数。
pub fn passthrough(args: Vec<OsString>, options: RuntimeOptions) -> Result<i32> {
    let jobs = settings::jobs(options.jobs)?;
    let automation = options.automation();
    automation.validate()?;
    let root = workspace::find_root()?;
    if automation.plan {
        return plan_passthrough(&args, &root, jobs, &automation);
    }
    // 即使看似只读的 Git 子命令也可能修改仓库，因此透传统一获取工作区锁。
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, &automation)?;
    let workspace = workspace::read(&root)?;
    let verbose = options.verbose || settings::passthrough_verbose()?;
    let results = map_repository_results(
        &workspace.repositories,
        jobs,
        &automation,
        "passthrough",
        &root,
        |repository, path| {
            if !git::is_repository(path) {
                return RepositoryResult::skipped(repository, "repository is not materialized");
            }
            match git::run_os_with_options(
                path,
                &args,
                false,
                git_execution_options(&automation, jobs == 1),
            ) {
                Ok(output) => passthrough_result(repository, &args, output),
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            }
        },
    )?;
    print_results(&results, verbose, &automation, "passthrough", &root)
}

/// 在用户精确选择的仓库子集中执行原生 Git 命令。
pub(super) fn exec(
    arguments: ExecArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let workspace = workspace::read(&root)?;
    let repositories = select_exec_repositories(&workspace, &arguments)?;
    let results = map_repository_results(
        &repositories,
        jobs,
        automation,
        "exec",
        &root,
        |repository, path| {
            if !git::is_repository(path) {
                return RepositoryResult::failed(repository, "repository is not materialized");
            }
            match git::run_os_with_options(
                path,
                &arguments.git_args,
                false,
                git_execution_options(automation, jobs == 1),
            ) {
                Ok(output) => passthrough_result(repository, &arguments.git_args, output),
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            }
        },
    )?;
    print_selected_results(&results, verbose, automation, "exec", &root)
}

/// 把 Git 子进程结果转换为统一的仓库级结果。
fn passthrough_result(
    repository: &RepositoryRecord,
    args: &[OsString],
    output: GitOutput,
) -> RepositoryResult {
    if is_nothing_to_commit(args, &output) {
        RepositoryResult::skipped_from_git(repository, "nothing to commit", output)
    } else {
        RepositoryResult::from_git(repository, output, "Git command completed", false)
    }
}

/// 识别 `git commit` 的“没有内容可提交”，将其视为跳过而非批量失败。
fn is_nothing_to_commit(args: &[OsString], output: &GitOutput) -> bool {
    if output.success || output.code != Some(1) || args.first().is_none_or(|arg| arg != "commit") {
        return false;
    }
    let message = format!("{}\n{}", output.stdout, output.stderr).to_ascii_lowercase();
    [
        "nothing to commit",
        "no changes added to commit",
        "nothing added to commit",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}
