//! Manifest-driven repository restoration.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, bail};
use indicatif::ProgressBar;

use super::super::{
    OperationProgress, finish_operation, git_execution_options, map_repository_results,
    set_operation_status, verify_apply_revision,
};
use super::support::reserve_clone_destination;
use crate::automation::AutomationOptions;
use crate::git::{self, CloneOptions, GitExecutionOptions};
use crate::model::{RepositoryRecord, WORKSPACE_FILE, now};
use crate::report::RepositoryResult;
use crate::workspace::{self, WorkspaceLock};

/// Clone missing repositories and reconcile declared remote configuration.
pub(in crate::commands) fn restore(
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::current_root()?;
    if !root.join(WORKSPACE_FILE).is_file() {
        bail!("restore requires {} in {}", WORKSPACE_FILE, root.display());
    }
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let mut manifest = workspace::read(&root)?;
    let records = manifest.repositories.clone();
    let progress = OperationProgress::new(&records, automation.is_machine());
    let results =
        map_repository_results(&records, jobs, automation, "restore", &root, |repository| {
            let bar = progress.bar(repository);
            restore_one(
                &root,
                repository,
                git_execution_options(automation, jobs == 1),
                bar.as_ref(),
            )
        })?;
    progress.finish();
    for (record, outcome) in manifest.repositories.iter_mut().zip(&results) {
        if outcome.was_synced() {
            record.synced_at = Some(now());
        }
    }
    if results.iter().any(RepositoryResult::was_synced) {
        workspace::write(&root, &mut manifest)?;
    }
    crate::report::print_operation_summary(&results, verbose, automation, "restore", &root)
}

/// Restore or validate one repository without aborting other repository operations.
pub(in crate::commands) fn restore_one(
    root: &Path,
    repository: &RepositoryRecord,
    execution: GitExecutionOptions,
    progress: Option<&ProgressBar>,
) -> RepositoryResult {
    set_operation_status(progress, "checking", 0);
    let target = root.join(&repository.directory);
    if target.exists() {
        if !git::is_repository(&target) {
            let result =
                RepositoryResult::failed(repository, "target exists but is not a Git repository");
            finish_operation(progress, &result);
            return result;
        }
        let result = match git::verify_declared_remotes(&target, repository).and_then(|()| {
            git::configure_declared_remotes(&target, repository, execution.allow_stdin)
        }) {
            Ok(()) => RepositoryResult::skipped(repository, "existing repository verified"),
            Err(error) => RepositoryResult::failed(repository, error.to_string()),
        };
        finish_operation(progress, &result);
        return result;
    }

    let Some(primary) = repository
        .remotes
        .iter()
        .find(|remote| remote.name == repository.primary_remote)
    else {
        let result = RepositoryResult::failed(repository, "primary remote is not declared");
        finish_operation(progress, &result);
        return result;
    };
    if let Err(error) = reserve_clone_destination(&target) {
        let result = RepositoryResult::failed(repository, error.to_string());
        finish_operation(progress, &result);
        return result;
    }
    set_operation_status(progress, "cloning", 0);
    let clone_progress = progress.cloned().map(|bar| {
        Arc::new(move |received: usize, total: usize| {
            let percentage = received.saturating_mul(100).checked_div(total).unwrap_or(0);
            bar.set_position(percentage as u64);
            bar.set_message(format!("receiving {received}/{total}"));
        }) as git::CloneProgress
    });
    if let Err(error) = git::clone_repository_with_options(
        &primary.fetch_url,
        &target,
        CloneOptions {
            remote_name: &repository.primary_remote,
            branch: Some(&repository.default_branch),
            depth: None,
            single_branch: true,
            progress: clone_progress,
            allow_stdin: execution.allow_stdin,
        },
        execution,
    ) {
        let result = RepositoryResult::failed(repository, error.to_string());
        finish_operation(progress, &result);
        return result;
    }
    set_operation_status(progress, "configuring", 98);
    if let Err(error) = git::configure_declared_remotes(&target, repository, execution.allow_stdin)
    {
        let result = RepositoryResult::failed(repository, error.to_string());
        finish_operation(progress, &result);
        return result;
    }
    let result = RepositoryResult::success(repository, "restored default branch", true);
    finish_operation(progress, &result);
    result
}
