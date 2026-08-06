use std::path::Path;

use anyhow::{Context, Result, bail};
use git2::build::CheckoutBuilder;
use git2::{BranchType, Repository};

use super::types::CheckoutTarget;

/// 安全切换到已经存在的本地分支。
pub fn checkout_local(path: &Path, branch_name: &str) -> Result<()> {
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    let branch = repository
        .find_branch(branch_name, BranchType::Local)
        .with_context(|| format!("failed to read local branch {branch_name}"))?;
    let commit = branch
        .get()
        .peel_to_commit()
        .with_context(|| format!("branch {branch_name} does not point to a commit"))?;
    let mut checkout = CheckoutBuilder::new();
    checkout.safe().recreate_missing(true);
    repository
        .checkout_tree(commit.as_object(), Some(&mut checkout))
        .with_context(|| format!("local changes prevent checkout of {branch_name}"))?;
    repository
        .set_head(&format!("refs/heads/{branch_name}"))
        .with_context(|| format!("failed to set HEAD to {branch_name}"))
}

/// 从远端引用创建同名 tracking 分支并切换过去。
pub fn checkout_remote(path: &Path, branch_name: &str, remote_branch: &str) -> Result<()> {
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    let remote = repository
        .find_branch(remote_branch, BranchType::Remote)
        .with_context(|| format!("failed to read remote branch {remote_branch}"))?;
    let commit = remote
        .get()
        .peel_to_commit()
        .with_context(|| format!("remote branch {remote_branch} does not point to a commit"))?;
    let mut local = repository
        .branch(branch_name, &commit, false)
        .with_context(|| format!("failed to create local branch {branch_name}"))?;
    local
        .set_upstream(Some(remote_branch))
        .with_context(|| format!("failed to track {remote_branch}"))?;
    let mut checkout = CheckoutBuilder::new();
    checkout.safe().recreate_missing(true);
    if let Err(error) = repository.checkout_tree(commit.as_object(), Some(&mut checkout)) {
        let _ = local.delete();
        return Err(error)
            .with_context(|| format!("local changes prevent checkout of branch {branch_name}"));
    }
    repository
        .set_head(&format!("refs/heads/{branch_name}"))
        .with_context(|| format!("failed to set HEAD to {branch_name}"))
}

/// 从显式或隐式起点创建新本地分支并安全 checkout。
pub fn create_and_checkout_branch(
    path: &Path,
    branch_name: &str,
    start_point: Option<&str>,
    remote_name: Option<&str>,
) -> Result<()> {
    validate_branch_name(branch_name)?;
    let full_name = format!("refs/heads/{branch_name}");
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    if repository
        .find_branch(branch_name, BranchType::Local)
        .is_ok()
    {
        bail!("branch already exists: {branch_name}");
    }

    let (commit, upstream) = match start_point {
        None => {
            let commit = repository
                .head()
                .context("cannot create a branch from an unborn or missing HEAD")?
                .peel_to_commit()
                .context("HEAD does not point to a commit")?;
            (commit, None)
        }
        Some(start) => resolve_create_start_point(&repository, start, remote_name)?,
    };

    let mut branch = repository
        .branch(branch_name, &commit, false)
        .with_context(|| format!("failed to create local branch {branch_name}"))?;
    if let Some(upstream) = upstream.as_deref()
        && let Err(error) = branch.set_upstream(Some(upstream))
    {
        let _ = branch.delete();
        return Err(error).with_context(|| format!("failed to track {upstream}"));
    }
    let mut checkout = CheckoutBuilder::new();
    checkout.safe().recreate_missing(true);
    if let Err(error) = repository.checkout_tree(commit.as_object(), Some(&mut checkout)) {
        let _ = branch.delete();
        return Err(error)
            .with_context(|| format!("local changes prevent checkout of branch {branch_name}"));
    }
    repository
        .set_head(&full_name)
        .with_context(|| format!("failed to set HEAD to {branch_name}"))
}

/// 按“本地优先、显式远端其次、全远端搜索最后”的规则解析 checkout。
pub fn checkout_target(path: &Path, branch: &str, remote: Option<&str>) -> Result<CheckoutTarget> {
    validate_branch_name(branch)?;
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    if repository.find_branch(branch, BranchType::Local).is_ok() {
        return Ok(CheckoutTarget::Local);
    }
    remote_tracking_target_in_repository(&repository, branch, remote)
}

/// Resolve a remote-tracking branch without falling back to a same-named local branch.
pub fn remote_tracking_target(
    path: &Path,
    branch: &str,
    remote: Option<&str>,
) -> Result<CheckoutTarget> {
    validate_branch_name(branch)?;
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    remote_tracking_target_in_repository(&repository, branch, remote)
}

/// 将分支、远端分支、tag 或提交名解析为创建分支的 commit。
fn resolve_create_start_point<'repo>(
    repository: &'repo Repository,
    start: &str,
    remote_name: Option<&str>,
) -> Result<(git2::Commit<'repo>, Option<String>)> {
    if let Some(remote_name) = remote_name {
        let remote_branch = format!("{remote_name}/{start}");
        let branch = repository
            .find_branch(&remote_branch, BranchType::Remote)
            .with_context(|| format!("remote branch does not exist: {remote_branch}"))?;
        let commit = branch
            .get()
            .peel_to_commit()
            .with_context(|| format!("remote branch {remote_branch} does not point to a commit"))?;
        return Ok((commit, Some(remote_branch)));
    }

    if let Ok(branch) = repository.find_branch(start, BranchType::Local) {
        let commit = branch
            .get()
            .peel_to_commit()
            .with_context(|| format!("local branch {start} does not point to a commit"))?;
        return Ok((commit, None));
    }

    match remote_tracking_target_in_repository(repository, start, None)? {
        CheckoutTarget::Remote(remote_branch) => {
            let branch = repository
                .find_branch(&remote_branch, BranchType::Remote)
                .with_context(|| format!("failed to read remote branch {remote_branch}"))?;
            let commit = branch.get().peel_to_commit().with_context(|| {
                format!("remote branch {remote_branch} does not point to a commit")
            })?;
            Ok((commit, Some(remote_branch)))
        }
        CheckoutTarget::Ambiguous(matches) => {
            bail!("start point is ambiguous: {}", matches.join(", "))
        }
        CheckoutTarget::Missing => {
            let object = repository
                .revparse_single(start)
                .with_context(|| format!("start point does not exist: {start}"))?;
            let commit = object
                .peel_to_commit()
                .with_context(|| format!("start point is not a commit: {start}"))?;
            Ok((commit, None))
        }
        CheckoutTarget::Local => unreachable!("only remote branches are inspected"),
    }
}

fn remote_tracking_target_in_repository(
    repository: &Repository,
    branch: &str,
    remote: Option<&str>,
) -> Result<CheckoutTarget> {
    let mut matches = Vec::new();
    let branches = repository
        .branches(Some(BranchType::Remote))
        .context("failed to enumerate remote branches")?;
    for branch_result in branches {
        let (candidate, _) = branch_result.context("failed to inspect remote branch")?;
        let Some(name) = candidate
            .name()
            .context("remote branch name is not UTF-8")?
        else {
            continue;
        };
        let Some((remote_name, logical_name)) = name.split_once('/') else {
            continue;
        };
        if logical_name == branch && remote.is_none_or(|selected| selected == remote_name) {
            matches.push(name.to_owned());
        }
    }
    matches.sort();
    matches.dedup();
    Ok(match matches.len() {
        0 => CheckoutTarget::Missing,
        1 => CheckoutTarget::Remote(matches.remove(0)),
        _ => CheckoutTarget::Ambiguous(matches),
    })
}

fn validate_branch_name(branch: &str) -> Result<()> {
    let full_name = format!("refs/heads/{branch}");
    if !git2::Reference::is_valid_name(&full_name) {
        bail!("invalid branch name: {branch}");
    }
    Ok(())
}
