//! Read-only Git repository inspection.

use std::path::Path;

use anyhow::{Context, Result, bail};
use git2::{BranchType, Repository, Status, StatusOptions};

use crate::model::RemoteRecord;

use super::remotes::portable_remote_url;
use super::types::{
    BranchKind, BranchSummary, ChangeCounts, RepositoryInfo, RepositoryRuntimeInfo,
    RepositoryRuntimeState, RepositoryStatusSummary, StagingState, UpstreamSummary,
};

/// 读取仓库默认分支、主要远端和可移植远端地址。
pub fn inspect(path: &Path) -> Result<RepositoryInfo> {
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    if repository.is_bare() {
        bail!(
            "bare repositories cannot be workspace modules: {}",
            path.display()
        );
    }
    let mut remotes = Vec::new();
    let names = repository.remotes().context("failed to list remotes")?;
    for name in names.iter().filter_map(|name| name.ok().flatten()) {
        let remote = repository
            .find_remote(name)
            .with_context(|| format!("failed to read remote {name}"))?;
        let Ok(url) = remote.url() else {
            continue;
        };
        remotes.push(RemoteRecord {
            name: name.to_owned(),
            fetch_url: portable_remote_url(url),
            push_url: remote.pushurl().ok().flatten().map(portable_remote_url),
        });
    }
    remotes.sort_by(|a, b| a.name.cmp(&b.name));
    if remotes.is_empty() {
        bail!("repository {} has no usable remotes", path.display());
    }
    let primary_remote = if remotes.iter().any(|remote| remote.name == "origin") {
        "origin".to_owned()
    } else {
        remotes[0].name.clone()
    };
    let default_branch = detect_default_branch(&repository, &primary_remote);
    Ok(RepositoryInfo {
        default_branch,
        primary_remote,
        remotes,
    })
}

/// 快速判断路径是否为可打开的非裸 Git 工作树。
pub fn is_repository(path: &Path) -> bool {
    Repository::open(path)
        .map(|repository| !repository.is_bare())
        .unwrap_or(false)
}

/// 返回当前分支名；detached HEAD 时返回短提交 ID。
pub fn current_branch_summary(path: &Path) -> Result<String> {
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    let head = match repository.head() {
        Ok(head) => head,
        Err(error) if error.code() == git2::ErrorCode::UnbornBranch => {
            return Ok("(unborn)".to_owned());
        }
        Err(error) => return Err(error.into()),
    };
    if head.is_branch() {
        return Ok(head.shorthand().unwrap_or("(unknown)").to_owned());
    }
    let short_id = head
        .target()
        .map(|oid| oid.to_string().chars().take(8).collect::<String>())
        .unwrap_or_else(|| "unknown".to_owned());
    Ok(format!("(detached:{short_id})"))
}

/// Return whether HEAD is unborn without relying on the human-readable branch summary.
pub fn head_is_unborn(path: &Path) -> Result<bool> {
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    match repository.head() {
        Ok(_) => Ok(false),
        Err(error) if error.code() == git2::ErrorCode::UnbornBranch => Ok(true),
        Err(error) => Err(error.into()),
    }
}

/// 容错收集仓库状态；异常被编码进状态而不是中断整个工作区。
pub fn repository_runtime_info(path: &Path) -> RepositoryRuntimeInfo {
    if !path.exists() {
        return unavailable_runtime(RepositoryRuntimeState::Missing);
    }
    let repository = match Repository::open(path) {
        Ok(repository) => repository,
        Err(error) if error.code() == git2::ErrorCode::NotFound => {
            return unavailable_runtime(RepositoryRuntimeState::NotGit);
        }
        Err(_) => return unavailable_runtime(RepositoryRuntimeState::Error),
    };
    let state = if repository.is_bare() {
        RepositoryRuntimeState::Bare
    } else {
        RepositoryRuntimeState::Available
    };
    let head = match repository.head() {
        Ok(head) => head,
        Err(error) if error.code() == git2::ErrorCode::UnbornBranch => {
            return RepositoryRuntimeInfo {
                state,
                current_branch: Some("(unborn)".to_owned()),
                head: None,
                local_branches: branch_count(&repository, BranchType::Local),
                remote_branches: branch_count(&repository, BranchType::Remote),
            };
        }
        Err(_) => return unavailable_runtime(RepositoryRuntimeState::Error),
    };
    let current_branch = if head.is_branch() {
        head.shorthand().ok().map(str::to_owned)
    } else {
        head.target().map(|oid| {
            format!(
                "(detached:{})",
                oid.to_string().chars().take(8).collect::<String>()
            )
        })
    };
    RepositoryRuntimeInfo {
        state,
        current_branch,
        head: head
            .target()
            .map(|oid| oid.to_string().chars().take(8).collect()),
        local_branches: branch_count(&repository, BranchType::Local),
        remote_branches: branch_count(&repository, BranchType::Remote),
    }
}

/// 构造无法读取 Git 细节时的统一空摘要。
fn unavailable_runtime(state: RepositoryRuntimeState) -> RepositoryRuntimeInfo {
    RepositoryRuntimeInfo {
        state,
        current_branch: None,
        head: None,
        local_branches: None,
        remote_branches: None,
    }
}

/// 统计指定类型分支；遍历失败时返回 None 表示数据不可用。
fn branch_count(repository: &Repository, branch_type: BranchType) -> Option<usize> {
    let branches = repository.branches(Some(branch_type)).ok()?;
    let mut count = 0;
    for result in branches {
        let (branch, actual_type) = result.ok()?;
        if actual_type == BranchType::Remote
            && branch.get().symbolic_target().ok().flatten().is_some()
        {
            continue;
        }
        count += 1;
    }
    Some(count)
}

/// 收集本地和/或远端分支，并附带提交信息供搜索排序。
pub fn branches(
    path: &Path,
    include_local: bool,
    include_remote: bool,
) -> Result<Vec<BranchSummary>> {
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    let current_reference = repository
        .head()
        .ok()
        .and_then(|head| head.name().ok().map(str::to_owned));
    let mut summaries = Vec::new();
    if include_local {
        collect_branches(
            &repository,
            BranchType::Local,
            current_reference.as_deref(),
            &mut summaries,
        )?;
    }
    if include_remote {
        collect_branches(
            &repository,
            BranchType::Remote,
            current_reference.as_deref(),
            &mut summaries,
        )?;
    }
    summaries.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| match (left.kind, right.kind) {
                (BranchKind::Local, BranchKind::Remote) => std::cmp::Ordering::Less,
                (BranchKind::Remote, BranchKind::Local) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            })
            .then_with(|| left.remote.cmp(&right.remote))
    });
    Ok(summaries)
}

/// 将 git2 分支迭代器转换为稳定的业务摘要。
fn collect_branches(
    repository: &Repository,
    branch_type: BranchType,
    current_reference: Option<&str>,
    summaries: &mut Vec<BranchSummary>,
) -> Result<()> {
    let branches = repository
        .branches(Some(branch_type))
        .context("failed to enumerate branches")?;
    for branch_result in branches {
        let (branch, actual_type) = branch_result.context("failed to inspect branch")?;
        let reference = branch.get();
        if actual_type == BranchType::Remote && reference.symbolic_target().ok().flatten().is_some()
        {
            continue;
        }
        let Some(full_name) = branch.name().context("branch name is not UTF-8")? else {
            continue;
        };
        let (name, remote) = if actual_type == BranchType::Remote {
            let Some((remote, name)) = full_name.split_once('/') else {
                continue;
            };
            (name.to_owned(), Some(remote.to_owned()))
        } else {
            (full_name.to_owned(), None)
        };
        let commit = reference
            .peel_to_commit()
            .with_context(|| format!("failed to read branch commit for {full_name}"))?;
        let commit_id = commit.id().to_string();
        summaries.push(BranchSummary {
            name,
            kind: if actual_type == BranchType::Local {
                BranchKind::Local
            } else {
                BranchKind::Remote
            },
            remote,
            is_current: actual_type == BranchType::Local
                && reference
                    .name()
                    .is_ok_and(|name| Some(name) == current_reference),
            commit_short: commit_id.chars().take(8).collect(),
            commit: commit_id,
            commit_time: commit.time().seconds(),
        });
    }
    Ok(())
}

/// 汇总工作树变更与本地引用计算出的 upstream 差异。
pub fn status_summary(path: &Path) -> Result<RepositoryStatusSummary> {
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    let branch = current_branch_summary(path)?;
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);
    let statuses = repository
        .statuses(Some(&mut options))
        .context("failed to inspect repository status")?;
    let mut changes = ChangeCounts::default();
    for entry in statuses.iter() {
        let status = entry.status();
        if status.contains(Status::CONFLICTED) {
            changes.conflicted += 1;
        } else if status.intersects(Status::INDEX_RENAMED | Status::WT_RENAMED) {
            changes.renamed += 1;
        } else if status.intersects(Status::INDEX_DELETED | Status::WT_DELETED) {
            changes.deleted += 1;
        } else if status.contains(Status::INDEX_NEW) {
            changes.added += 1;
        } else if status.contains(Status::WT_NEW) {
            changes.untracked += 1;
        } else if status.intersects(
            Status::INDEX_MODIFIED
                | Status::WT_MODIFIED
                | Status::INDEX_TYPECHANGE
                | Status::WT_TYPECHANGE,
        ) {
            changes.modified += 1;
        }
    }
    let upstream = upstream_summary(&repository)?;
    Ok(RepositoryStatusSummary {
        branch,
        changes,
        upstream,
    })
}

/// Inspect whether a repository has index changes, stageable working-tree changes, or conflicts.
///
/// Conflicts count as both staged and stageable so callers cannot misclassify an unmerged index
/// as an ordinary no-op. The controlled commands then apply their own conservative policy.
pub fn staging_state(path: &Path) -> Result<StagingState> {
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);
    let statuses = repository
        .statuses(Some(&mut options))
        .context("failed to inspect repository staging state")?;
    let mut state = StagingState::default();
    for entry in statuses.iter() {
        let status = entry.status();
        if status.contains(Status::CONFLICTED) {
            state.has_conflicts = true;
            state.has_staged_changes = true;
            state.has_worktree_changes = true;
        }
        if status.intersects(
            Status::INDEX_NEW
                | Status::INDEX_MODIFIED
                | Status::INDEX_DELETED
                | Status::INDEX_RENAMED
                | Status::INDEX_TYPECHANGE,
        ) {
            state.has_staged_changes = true;
        }
        if status.intersects(
            Status::WT_NEW
                | Status::WT_MODIFIED
                | Status::WT_DELETED
                | Status::WT_RENAMED
                | Status::WT_TYPECHANGE,
        ) {
            state.has_worktree_changes = true;
        }
    }
    Ok(state)
}

/// Return whether Git is completing a merge, rebase, cherry-pick, revert, or similar sequence.
pub fn operation_in_progress(path: &Path) -> Result<bool> {
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    Ok(repository.state() != git2::RepositoryState::Clean)
}

/// 通过 merge-base 图关系计算 ahead/behind，不访问网络。
fn upstream_summary(repository: &Repository) -> Result<UpstreamSummary> {
    let head = match repository.head() {
        Ok(head) => head,
        Err(error) if error.code() == git2::ErrorCode::UnbornBranch => {
            return Ok(UpstreamSummary::None);
        }
        Err(error) => return Err(error.into()),
    };
    if !head.is_branch() {
        return Ok(UpstreamSummary::None);
    }
    let Ok(branch_name) = head.shorthand() else {
        return Ok(UpstreamSummary::None);
    };
    let branch = repository
        .find_branch(branch_name, BranchType::Local)
        .with_context(|| format!("failed to read local branch {branch_name}"))?;
    let upstream = match branch.upstream() {
        Ok(upstream) => upstream,
        Err(error) if error.code() == git2::ErrorCode::NotFound => {
            return Ok(UpstreamSummary::None);
        }
        Err(error) => return Err(error.into()),
    };
    let Some(local_oid) = head.target() else {
        return Ok(UpstreamSummary::None);
    };
    let Some(upstream_oid) = upstream.get().target() else {
        return Ok(UpstreamSummary::None);
    };
    let (ahead, behind) = repository
        .graph_ahead_behind(local_oid, upstream_oid)
        .context("failed to compare branch with upstream")?;
    Ok(match (ahead, behind) {
        (0, 0) => UpstreamSummary::UpToDate,
        (ahead, 0) => UpstreamSummary::Ahead(ahead),
        (0, behind) => UpstreamSummary::Behind(behind),
        (ahead, behind) => UpstreamSummary::Diverged { ahead, behind },
    })
}

/// 优先读取远端 HEAD，再回退到常见分支名和当前分支。
fn detect_default_branch(repository: &Repository, primary_remote: &str) -> String {
    let remote_head = format!("refs/remotes/{primary_remote}/HEAD");
    if let Ok(reference) = repository.find_reference(&remote_head)
        && let Ok(Some(target)) = reference.symbolic_target()
        && let Some(branch) = target.strip_prefix(&format!("refs/remotes/{primary_remote}/"))
    {
        return branch.to_owned();
    }
    for branch in ["main", "master"] {
        if repository
            .find_reference(&format!("refs/heads/{branch}"))
            .is_ok()
        {
            return branch.to_owned();
        }
    }
    if let Ok(head) = repository.head()
        && head.is_branch()
        && let Ok(branch) = head.shorthand()
    {
        return branch.to_owned();
    }
    "main".to_owned()
}
