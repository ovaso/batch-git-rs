//! Branch checkout and merge operations.

use anyhow::{Result, bail};
use serde_json::json;

use super::{git_execution_options, map_repository_results, merge_settings, verify_apply_revision};
use crate::automation::{self, AutomationOptions};
use crate::cli::{CheckoutArgs, MergeArgs};
use crate::git::{self, CheckoutTarget, UpstreamSummary};
use crate::model::now;
use crate::report::{RepositoryResult, print_checkout_summary, print_selected_results};
use crate::settings;
use crate::workspace::{self, WorkspaceLock};

/// 在各仓库解析并安全切换目标分支，缺少分支时允许正常跳过。
pub(super) fn checkout(
    arguments: CheckoutArgs,
    jobs: usize,
    _verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let current_feature_branch = settings::current_feature_branch()?;
    let feature_branch = if arguments.feature {
        match current_feature_branch.as_deref() {
            Some(branch) => Some(branch),
            None => return feature_branch_unset(automation, "checkout"),
        }
    } else {
        None
    };
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let manifest = workspace::read(&root)?;
    if arguments.create && arguments.from.is_none() && arguments.remote.is_some() {
        bail!("--remote requires --from when used with checkout -b");
    }
    let remote = if arguments.create && arguments.from.is_none() {
        None
    } else {
        settings::checkout_remote(arguments.remote)
    };
    let results = map_repository_results(
        &manifest.repositories,
        jobs,
        automation,
        "checkout",
        &root,
        |repository| {
            let path = root.join(&repository.directory);
            if !git::is_repository(&path) {
                return RepositoryResult::failed(
                    repository,
                    "repository is not materialized; run restore",
                );
            }
            let branch = arguments
                .branch
                .as_deref()
                .or(feature_branch)
                .unwrap_or(&repository.default_branch);
            if arguments.create {
                return match git::create_and_checkout_branch(
                    &path,
                    branch,
                    arguments.from.as_deref(),
                    remote.as_deref(),
                ) {
                    Ok(()) => RepositoryResult::success(
                        repository,
                        format!("created branch {branch}"),
                        false,
                    ),
                    Err(error) => RepositoryResult::failed(repository, error.to_string()),
                };
            }
            let selected_remote = if arguments.default {
                Some(repository.primary_remote.as_str())
            } else {
                remote.as_deref()
            };
            let target = match git::checkout_target(&path, branch, selected_remote) {
                Ok(target) => target,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            match target {
                CheckoutTarget::Missing => {
                    RepositoryResult::skipped(repository, format!("branch {branch} does not exist"))
                }
                CheckoutTarget::Ambiguous(matches) => RepositoryResult::failed(
                    repository,
                    format!("branch is ambiguous: {}", matches.join(", ")),
                ),
                CheckoutTarget::Local => match git::checkout_local(&path, branch) {
                    Ok(()) => {
                        RepositoryResult::success(repository, "checked out local branch", false)
                    }
                    Err(error) => RepositoryResult::failed(repository, error.to_string()),
                },
                CheckoutTarget::Remote(remote_branch) => {
                    match git::checkout_remote(&path, branch, &remote_branch) {
                        Ok(()) => RepositoryResult::success(
                            repository,
                            format!("created tracking branch from {remote_branch}"),
                            false,
                        ),
                        Err(error) => RepositoryResult::failed(repository, error.to_string()),
                    }
                }
            }
        },
    )?;
    let branches = manifest
        .repositories
        .iter()
        .map(|repository| {
            let path = root.join(&repository.directory);
            if !path.exists() {
                "(missing)".to_owned()
            } else if !git::is_repository(&path) {
                "(not-git)".to_owned()
            } else {
                git::current_branch_summary(&path).unwrap_or_else(|_| "(unknown)".to_owned())
            }
        })
        .collect::<Vec<_>>();
    let default_branches = manifest
        .repositories
        .iter()
        .map(|repository| repository.default_branch.clone())
        .collect::<Vec<_>>();
    print_checkout_summary(
        &results,
        &branches,
        &default_branches,
        current_feature_branch.as_deref(),
        automation,
        "checkout",
        &root,
    )
}

/// 将指定源分支合入每个仓库当前分支，可选先快进当前分支。
pub(super) fn merge(
    arguments: MergeArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let merge_settings = merge_settings(&arguments)?;
    let update_current = merge_settings.update_current;
    let refresh_source = merge_settings.refresh_source;
    let MergeArgs {
        update_current: _,
        no_update_current: _,
        refresh_source: _,
        no_refresh_source: _,
        remote,
        default,
        feature,
        branch,
    } = arguments;
    let feature_branch = if feature {
        match settings::current_feature_branch()? {
            Some(branch) => Some(branch),
            None => return feature_branch_unset(automation, "merge"),
        }
    } else {
        None
    };
    let shared_branch = branch.as_deref().or(feature_branch.as_deref());
    let selected_remote = if default {
        None
    } else {
        settings::checkout_remote(remote)
    };
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let mut manifest = workspace::read(&root)?;
    let results = map_repository_results(
        &manifest.repositories,
        jobs,
        automation,
        "merge",
        &root,
        |repository| {
            let branch = if default {
                repository.default_branch.as_str()
            } else {
                shared_branch.expect("clap requires a branch, --feature, or --default")
            };
            let path = root.join(&repository.directory);
            if !git::is_repository(&path) {
                return RepositoryResult::failed(
                    repository,
                    "repository is not materialized; run restore",
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
            if status.branch == branch {
                return RepositoryResult::skipped(repository, "source is the current branch");
            }

            if !update_current {
                match status.upstream {
                    UpstreamSummary::Behind(count) => {
                        return RepositoryResult::failed(
                            repository,
                            format!(
                                "current branch is behind upstream by {count} commit(s); run pull or retry merge with --update-current"
                            ),
                        );
                    }
                    UpstreamSummary::Diverged { ahead, behind } => {
                        return RepositoryResult::failed(
                            repository,
                            format!(
                                "current branch has diverged from upstream: ahead {ahead}, behind {behind}; resolve the divergence before merging"
                            ),
                        );
                    }
                    _ => {}
                }
            }

            let mut current_updated = false;
            if update_current && !matches!(status.upstream, UpstreamSummary::None) {
                let output = match git::run_with_options(
                    &path,
                    ["pull", "--ff-only"],
                    true,
                    git_execution_options(automation, jobs == 1),
                ) {
                    Ok(output) => output,
                    Err(error) => return RepositoryResult::failed(repository, error.to_string()),
                };
                if !output.success {
                    return RepositoryResult::from_git(
                        repository,
                        output,
                        "current branch updated",
                        false,
                    );
                }
                current_updated = true;
            }

            let remote = if default {
                Some(repository.primary_remote.as_str())
            } else {
                selected_remote.as_deref()
            };
            let source = if refresh_source {
                if let Err(error) = git::configure_declared_remotes(
                    &path,
                    repository,
                    git_execution_options(automation, jobs == 1).allow_stdin,
                ) {
                    return RepositoryResult::failed(repository, error.to_string());
                }
                let output = match git::fetch_all_with_options(
                    &path,
                    git_execution_options(automation, jobs == 1),
                ) {
                    Ok(output) => output,
                    Err(error) => return RepositoryResult::failed(repository, error.to_string()),
                };
                if !output.success {
                    return RepositoryResult::from_git(
                        repository,
                        output,
                        "source remote refs refreshed",
                        false,
                    );
                }
                let refresh_remote = remote.or(Some(repository.primary_remote.as_str()));
                match git::remote_tracking_target(&path, branch, refresh_remote) {
                    Ok(CheckoutTarget::Remote(remote_branch)) => remote_branch,
                    Ok(CheckoutTarget::Missing) => {
                        return RepositoryResult::skipped(
                            repository,
                            format!("remote source branch {branch} does not exist"),
                        );
                    }
                    Ok(CheckoutTarget::Ambiguous(matches)) => {
                        return RepositoryResult::failed(
                            repository,
                            format!("branch is ambiguous: {}", matches.join(", ")),
                        );
                    }
                    Ok(CheckoutTarget::Local) => {
                        unreachable!("remote-only resolution cannot be local")
                    }
                    Err(error) => return RepositoryResult::failed(repository, error.to_string()),
                }
            } else {
                match git::checkout_target(&path, branch, remote) {
                    Ok(CheckoutTarget::Local) => branch.to_owned(),
                    Ok(CheckoutTarget::Remote(remote_branch)) => remote_branch,
                    Ok(CheckoutTarget::Missing) => {
                        return RepositoryResult::skipped(
                            repository,
                            format!("branch {branch} does not exist"),
                        );
                    }
                    Ok(CheckoutTarget::Ambiguous(matches)) => {
                        return RepositoryResult::failed(
                            repository,
                            format!("branch is ambiguous: {}", matches.join(", ")),
                        );
                    }
                    Err(error) => return RepositoryResult::failed(repository, error.to_string()),
                }
            };
            match git::run_with_options(
                &path,
                ["merge", "--no-edit", source.as_str()],
                true,
                git_execution_options(automation, jobs == 1),
            ) {
                Ok(output) => RepositoryResult::from_git(
                    repository,
                    output,
                    format!("merged {source} into {}", status.branch),
                    current_updated || refresh_source,
                ),
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            }
        },
    )?;
    for (record, outcome) in manifest.repositories.iter_mut().zip(&results) {
        if outcome.was_synced() {
            record.synced_at = Some(now());
        }
    }
    if results.iter().any(RepositoryResult::was_synced) {
        workspace::write(&root, &mut manifest)?;
    }
    print_selected_results(&results, verbose, automation, "merge", &root)
}

/// Render an invocation-level no-op without pretending it is a per-repository batch result.
fn feature_branch_unset(automation: &AutomationOptions, operation: &str) -> Result<i32> {
    let data = json!({
        "status": "skipped",
        "reason_code": "feature_branch_unset",
        "detail": format!("CURRENT_FEATURE_BRANCH is not set; nothing to {operation}."),
    });
    if automation.is_machine() {
        automation::emit_data(automation, operation, None, 0, &data)?;
    } else {
        println!("CURRENT_FEATURE_BRANCH is not set; nothing to {operation}");
    }
    Ok(0)
}
