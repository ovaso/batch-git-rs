//! Fetch, sync, pull, and push orchestration.

use super::*;

/// 对所有已物化仓库执行 fetch/prune，不改变工作树。
pub(super) fn fetch(jobs: usize, verbose: bool, automation: &AutomationOptions) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let mut manifest = workspace::read(&root)?;
    let records = manifest.repositories.clone();
    let progress = OperationProgress::new(&records, automation.is_machine());
    let results =
        map_repository_results(&records, jobs, automation, "fetch", &root, |repository| {
            let bar = progress.bar(repository);
            set_operation_status(bar.as_ref(), "checking", 0);
            let path = root.join(&repository.directory);
            if !git::is_repository(&path) {
                let result = RepositoryResult::failed(
                    repository,
                    "repository is not materialized; run restore",
                );
                finish_operation(bar.as_ref(), &result);
                return result;
            }
            set_operation_status(bar.as_ref(), "configuring", 0);
            if let Err(error) = git::configure_declared_remotes(
                &path,
                repository,
                git_execution_options(automation, jobs == 1).allow_stdin,
            ) {
                let result = RepositoryResult::failed(repository, error.to_string());
                finish_operation(bar.as_ref(), &result);
                return result;
            }
            set_operation_status(bar.as_ref(), "fetching", 0);
            let result = match git::fetch_all_with_options(
                &path,
                git_execution_options(automation, jobs == 1),
            ) {
                Ok(output) => {
                    RepositoryResult::from_git(repository, output, "remote refs updated", true)
                }
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            };
            finish_operation(bar.as_ref(), &result);
            result
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
    crate::report::print_operation_summary(&results, verbose, automation, "fetch", &root)
}

/// 解析用户选择后执行可供无人值守使用的 restore + fetch。
pub(super) fn sync(
    arguments: SyncArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let mut manifest = workspace::read(&root)?;
    let records = crate::selector::select(
        &manifest,
        &arguments.selectors,
        &arguments.matches,
        arguments.all,
    )?;
    run_sync(&root, &mut manifest, &records, jobs, verbose, automation)
}

/// 执行已确定范围的同步；供命令行和 schedule 共用。
pub(crate) fn run_sync(
    root: &Path,
    manifest: &mut Workspace,
    records: &[RepositoryRecord],
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    run_sync_named(root, manifest, records, jobs, verbose, automation, "sync")
}

/// Execute sync with the externally visible operation name supplied by a parent workflow.
pub(crate) fn run_sync_named(
    root: &Path,
    manifest: &mut Workspace,
    records: &[RepositoryRecord],
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
    command: &str,
) -> Result<i32> {
    let progress = OperationProgress::new(records, automation.is_machine());
    let results = map_repository_results(records, jobs, automation, command, root, |repository| {
        let bar = progress.bar(repository);
        set_operation_status(bar.as_ref(), "checking", 0);
        let restore_result = restore_one(
            root,
            repository,
            git_execution_options(automation, jobs == 1),
            None,
        );
        if restore_result.is_failed() {
            finish_operation(bar.as_ref(), &restore_result);
            return restore_result;
        }
        let restored = restore_result.was_synced();
        set_operation_status(bar.as_ref(), "fetching", 0);
        let path = root.join(&repository.directory);
        let result = match git::fetch_all_with_options(
            &path,
            git_execution_options(automation, jobs == 1),
        ) {
            Ok(output) => RepositoryResult::from_git(
                repository,
                output,
                if restored {
                    "restored and remote refs updated"
                } else {
                    "remote refs updated"
                },
                true,
            ),
            Err(error) => RepositoryResult::failed(repository, error.to_string()),
        };
        finish_operation(bar.as_ref(), &result);
        result
    })?;
    progress.finish();

    let timestamp = now();
    let mut changed = false;
    for (selected, outcome) in records.iter().zip(&results) {
        if !outcome.was_synced() {
            continue;
        }
        if let Some(record) = manifest
            .repositories
            .iter_mut()
            .find(|record| record.name == selected.name)
        {
            record.synced_at = Some(timestamp.clone());
            changed = true;
        }
    }
    if changed {
        workspace::write(root, manifest)?;
    }
    crate::report::print_operation_summary(&results, verbose, automation, command, root)
}

/// 解析用户选择后执行安全的 fast-forward-only pull。
pub(super) fn pull(
    arguments: SyncArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let mut manifest = workspace::read(&root)?;
    let records = crate::selector::select(
        &manifest,
        &arguments.selectors,
        &arguments.matches,
        arguments.all,
    )?;
    run_pull(&root, &mut manifest, &records, jobs, verbose, automation)
}

/// 对选中仓库检查工作树、分支和 upstream 后执行 fast-forward pull。
pub(crate) fn run_pull(
    root: &Path,
    manifest: &mut Workspace,
    records: &[RepositoryRecord],
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    run_pull_named(root, manifest, records, jobs, verbose, automation, "pull")
}

/// Execute pull with the externally visible operation name supplied by a parent workflow.
pub(crate) fn run_pull_named(
    root: &Path,
    manifest: &mut Workspace,
    records: &[RepositoryRecord],
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
    command: &str,
) -> Result<i32> {
    let results = map_repository_results(records, jobs, automation, command, root, |repository| {
        let path = root.join(&repository.directory);
        if !git::is_repository(&path) {
            return RepositoryResult::failed(
                repository,
                "repository is not materialized; run sync or restore",
            );
        }
        let status = match git::status_summary(&path) {
            Ok(status) => status,
            Err(error) => return RepositoryResult::failed(repository, error.to_string()),
        };
        if status.branch.starts_with("(detached:") || status.branch == "(unborn)" {
            return RepositoryResult::failed(repository, "current HEAD is not a local branch");
        }
        if status.changes.total() != 0 {
            return RepositoryResult::failed(repository, "working tree is not clean");
        }
        if matches!(status.upstream, UpstreamSummary::None) {
            return RepositoryResult::failed(repository, "current branch has no upstream");
        }
        match git::run_with_options(
            &path,
            ["pull", "--ff-only"],
            true,
            git_execution_options(automation, false),
        ) {
            Ok(output) => RepositoryResult::from_git(
                repository,
                output,
                format!("branch {} updated with fast-forward only", status.branch),
                true,
            ),
            Err(error) => RepositoryResult::failed(repository, error.to_string()),
        }
    })?;

    let timestamp = now();
    let mut changed = false;
    for (selected, outcome) in records.iter().zip(&results) {
        if !outcome.was_synced() {
            continue;
        }
        if let Some(record) = manifest
            .repositories
            .iter_mut()
            .find(|record| record.name == selected.name)
        {
            record.synced_at = Some(timestamp.clone());
            changed = true;
        }
    }
    if changed {
        workspace::write(root, manifest)?;
    }
    crate::report::print_operation_summary(&results, verbose, automation, command, root)
}

/// 推送各仓库当前分支，默认不创建远端分支且永不 force push。
pub(super) fn push(
    arguments: PushArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let manifest = workspace::read(&root)?;
    let records = crate::selector::select(
        &manifest,
        &arguments.selection.selectors,
        &arguments.selection.matches,
        arguments.selection.all,
    )?;
    let results =
        map_repository_results(&records, jobs, automation, "push", &root, |repository| {
            let path = root.join(&repository.directory);
            if !git::is_repository(&path) {
                return RepositoryResult::failed(
                    repository,
                    "repository is not materialized; run sync or restore",
                );
            }
            let status = match git::status_summary(&path) {
                Ok(status) => status,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            if status.branch.starts_with("(detached:") || status.branch == "(unborn)" {
                return RepositoryResult::failed(repository, "current HEAD is not a local branch");
            }
            let upstream_target = match git::upstream_push_target(&path, &status.branch) {
                Ok(target) => target,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };

            match status.upstream {
                UpstreamSummary::None if upstream_target.is_some() => {
                    let (remote, merge_ref) =
                        upstream_target.as_ref().expect("target checked above");
                    push_existing_upstream(
                        repository,
                        &path,
                        &status.branch,
                        remote,
                        merge_ref,
                        arguments.dry_run,
                        git_execution_options(automation, jobs == 1),
                    )
                }
                UpstreamSummary::None if !arguments.set_upstream => {
                    RepositoryResult::skipped(repository, "current branch has no upstream")
                }
                UpstreamSummary::None => {
                    let remote = arguments
                        .remote
                        .as_deref()
                        .unwrap_or(&repository.primary_remote);
                    let mut git_arguments = vec!["push"];
                    if arguments.dry_run {
                        git_arguments.push("--dry-run");
                    }
                    git_arguments.extend(["--set-upstream", remote, "HEAD"]);
                    match git::run_with_options(
                        &path,
                        git_arguments,
                        true,
                        git_execution_options(automation, jobs == 1),
                    ) {
                        Ok(output) => RepositoryResult::from_git(
                            repository,
                            output,
                            if arguments.dry_run {
                                format!(
                                    "would push {} to {remote} and configure upstream",
                                    status.branch
                                )
                            } else {
                                format!(
                                    "pushed {} to {remote} and configured upstream",
                                    status.branch
                                )
                            },
                            false,
                        ),
                        Err(error) => RepositoryResult::failed(repository, error.to_string()),
                    }
                }
                UpstreamSummary::UpToDate => {
                    RepositoryResult::skipped(repository, "nothing to push")
                }
                UpstreamSummary::Behind(count) => RepositoryResult::skipped(
                    repository,
                    format!("branch is behind upstream by {count} commit(s)"),
                ),
                UpstreamSummary::Diverged { ahead, behind } => RepositoryResult::failed(
                    repository,
                    format!("branch has diverged from upstream: ahead {ahead}, behind {behind}"),
                ),
                UpstreamSummary::Ahead(_) => {
                    let Some((remote, merge_ref)) = upstream_target.as_ref() else {
                        return RepositoryResult::failed(
                            repository,
                            "current branch has no configured upstream target",
                        );
                    };
                    push_existing_upstream(
                        repository,
                        &path,
                        &status.branch,
                        remote,
                        merge_ref,
                        arguments.dry_run,
                        git_execution_options(automation, jobs == 1),
                    )
                }
            }
        })?;
    print_push_summary(&results, verbose, automation, "push", &root)
}

/// 按已有 upstream 配置推送，先拒绝分叉等不安全状态。
fn push_existing_upstream(
    repository: &RepositoryRecord,
    path: &Path,
    branch: &str,
    remote: &str,
    merge_ref: &str,
    dry_run: bool,
    execution: GitExecutionOptions,
) -> RepositoryResult {
    let refspec = format!("HEAD:{merge_ref}");
    let mut git_arguments = vec!["push"];
    if dry_run {
        git_arguments.push("--dry-run");
    }
    git_arguments.extend([remote, refspec.as_str()]);
    match git::run_with_options(path, git_arguments, true, execution) {
        Ok(output) => RepositoryResult::from_git(
            repository,
            output,
            format!("pushed {branch}{}", if dry_run { " (dry run)" } else { "" }),
            false,
        ),
        Err(error) => RepositoryResult::failed(repository, error.to_string()),
    }
}
