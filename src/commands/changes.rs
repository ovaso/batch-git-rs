//! Index and local history operations.

use std::path::Path;

use anyhow::{Result, bail};

use super::{git_execution_options, map_repository_results, verify_apply_revision};
use crate::automation::AutomationOptions;
use crate::cli::{CommitArgs, SyncArgs};
use crate::git;
use crate::model::RepositoryRecord;
use crate::report::{RepositoryResult, print_selected_results};
use crate::workspace::{self, WorkspaceLock};

/// Stage every tracked, untracked, and deleted path in the selected repositories.
pub(super) fn add(
    arguments: SyncArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let execution = git_execution_options(automation, false);
    run_selected_local_operation(
        arguments,
        jobs,
        verbose,
        automation,
        "add",
        |repository, path| {
            let state = match git::staging_state(path) {
                Ok(state) => state,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            if state.has_conflicts {
                return RepositoryResult::failed(
                    repository,
                    "repository has unresolved conflicts; use exec with an explicit pathspec to stage resolutions",
                );
            }
            if !state.has_worktree_changes {
                return RepositoryResult::skipped(repository, "nothing to stage");
            }
            match git::run_with_options(path, ["add", "--all", "--", ":/"], true, execution) {
                Ok(output) => RepositoryResult::from_git(
                    repository,
                    output,
                    "all working-tree changes staged",
                    false,
                ),
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            }
        },
    )
}

/// Commit only content that was already present in each selected repository's index.
pub(super) fn commit(
    arguments: CommitArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let CommitArgs { selection, message } = arguments;
    let execution = git_execution_options(automation, jobs == 1);
    run_selected_local_operation(
        selection,
        jobs,
        verbose,
        automation,
        "commit",
        move |repository, path| {
            let branch = match git::current_branch_summary(path) {
                Ok(branch) => branch,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            if branch.starts_with("(detached:") {
                return RepositoryResult::failed(
                    repository,
                    "current HEAD is detached; checkout a local branch before committing",
                );
            }
            let state = match git::staging_state(path) {
                Ok(state) => state,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            if state.has_conflicts {
                return RepositoryResult::failed(repository, "repository has unresolved conflicts");
            }
            match git::operation_in_progress(path) {
                Ok(true) => {
                    return RepositoryResult::failed(
                        repository,
                        "repository operation is in progress; use an explicit Git workflow to continue it",
                    );
                }
                Ok(false) => {}
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            }
            if !state.has_staged_changes {
                return RepositoryResult::skipped(repository, "nothing to commit");
            }
            match git::run_with_options(path, ["commit", "-m", message.as_str()], true, execution) {
                Ok(output) => RepositoryResult::from_git(
                    repository,
                    output,
                    "committed staged changes",
                    false,
                ),
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            }
        },
    )
}

pub(super) fn validate_commit_message(message: &str) -> Result<()> {
    if message.trim().is_empty() {
        bail!("commit message cannot be empty");
    }
    Ok(())
}

/// Restore each selected index to HEAD without updating any working-tree file.
pub(super) fn unstage(
    arguments: SyncArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let execution = git_execution_options(automation, false);
    run_selected_local_operation(
        arguments,
        jobs,
        verbose,
        automation,
        "unstage",
        |repository, path| {
            let state = match git::staging_state(path) {
                Ok(state) => state,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            if !state.has_staged_changes {
                return RepositoryResult::skipped(repository, "nothing to unstage");
            }
            let head_is_unborn = match git::head_is_unborn(path) {
                Ok(head_is_unborn) => head_is_unborn,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            let output = if head_is_unborn {
                // `git restore --staged` requires HEAD. Emptying the unborn index has the same
                // unstage-all result and leaves every working-tree file untouched.
                git::run_with_options(path, ["read-tree", "--empty"], true, execution)
            } else {
                git::run_with_options(path, ["restore", "--staged", "--", ":/"], true, execution)
            };
            match output {
                Ok(output) => RepositoryResult::from_git(
                    repository,
                    output,
                    "staged changes removed; working tree preserved",
                    false,
                ),
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            }
        },
    )
}

/// Resolve one standard repository selection and execute an index/local-history operation.
fn run_selected_local_operation<F>(
    arguments: SyncArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
    command: &str,
    operation: F,
) -> Result<i32>
where
    F: Fn(&RepositoryRecord, &Path) -> RepositoryResult + Sync + Send,
{
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let manifest = workspace::read(&root)?;
    let records = crate::selector::select(
        &manifest,
        &arguments.selectors,
        &arguments.matches,
        arguments.all,
    )?;
    let results =
        map_repository_results(&records, jobs, automation, command, &root, |repository| {
            let path = root.join(&repository.directory);
            if !git::is_repository(&path) {
                return RepositoryResult::failed(
                    repository,
                    "repository is not materialized; run sync or restore",
                );
            }
            operation(repository, &path)
        })?;
    print_selected_results(&results, verbose, automation, command, &root)
}
