//! Git repository inspection through `git2` and compatibility-sensitive operations through Git.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use git2::build::CheckoutBuilder;
use git2::{BranchType, Repository, Status, StatusOptions};
use url::Url;

use crate::model::{RemoteRecord, RepositoryRecord};

const ROUTING_ENVIRONMENT: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_SHALLOW_FILE",
    "GIT_QUARANTINE_PATH",
    "GIT_PREFIX",
    "GIT_INTERNAL_SUPER_PREFIX",
    "GIT_IMPLICIT_WORK_TREE",
];

#[derive(Debug)]
pub struct GitOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug)]
pub struct RepositoryInfo {
    pub default_branch: String,
    pub primary_remote: String,
    pub remotes: Vec<RemoteRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryRuntimeState {
    Available,
    Missing,
    NotGit,
    Bare,
    Error,
}

impl RepositoryRuntimeState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Missing => "missing",
            Self::NotGit => "not-git",
            Self::Bare => "bare",
            Self::Error => "error",
        }
    }
}

#[derive(Debug)]
pub struct RepositoryRuntimeInfo {
    pub state: RepositoryRuntimeState,
    pub current_branch: Option<String>,
    pub head: Option<String>,
    pub local_branches: Option<usize>,
    pub remote_branches: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchKind {
    Local,
    Remote,
}

impl BranchKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

#[derive(Debug)]
pub struct BranchSummary {
    pub name: String,
    pub kind: BranchKind,
    pub remote: Option<String>,
    pub is_current: bool,
    pub commit: String,
    pub commit_short: String,
    pub commit_time: i64,
}

#[derive(Debug)]
pub enum CheckoutTarget {
    Local,
    Remote(String),
    Missing,
    Ambiguous(Vec<String>),
}

pub struct CloneOptions<'a> {
    pub remote_name: &'a str,
    pub branch: Option<&'a str>,
    pub depth: Option<usize>,
    pub single_branch: bool,
    pub progress: Option<CloneProgress>,
    pub allow_stdin: bool,
}

pub type CloneProgress = Arc<dyn Fn(usize, usize) + Send + Sync>;

#[derive(Debug, Default)]
pub struct ChangeCounts {
    pub modified: usize,
    pub added: usize,
    pub deleted: usize,
    pub renamed: usize,
    pub untracked: usize,
    pub conflicted: usize,
}

impl ChangeCounts {
    pub fn total(&self) -> usize {
        self.modified + self.added + self.deleted + self.renamed + self.untracked + self.conflicted
    }

    pub fn compact(&self) -> String {
        let values = [
            ("M", self.modified),
            ("A", self.added),
            ("D", self.deleted),
            ("R", self.renamed),
            ("?", self.untracked),
            ("U", self.conflicted),
        ]
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(label, count)| format!("{label}{count}"))
        .collect::<Vec<_>>();
        if values.is_empty() {
            "-".to_owned()
        } else {
            values.join(" ")
        }
    }
}

#[derive(Debug)]
pub enum UpstreamSummary {
    UpToDate,
    Ahead(usize),
    Behind(usize),
    Diverged { ahead: usize, behind: usize },
    None,
}

impl UpstreamSummary {
    pub fn label(&self) -> String {
        match self {
            Self::UpToDate => "up-to-date".to_owned(),
            Self::Ahead(count) => format!("ahead {count}"),
            Self::Behind(count) => format!("behind {count}"),
            Self::Diverged { ahead, behind } => format!("ahead {ahead}, behind {behind}"),
            Self::None => "no-upstream".to_owned(),
        }
    }
}

#[derive(Debug)]
pub struct RepositoryStatusSummary {
    pub branch: String,
    pub changes: ChangeCounts,
    pub upstream: UpstreamSummary,
}

pub fn run<I, S>(directory: &Path, args: I, managed: bool, allow_stdin: bool) -> Result<GitOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command.current_dir(directory).args(args);
    if let Some(timezone) = crate::settings::schedule_timezone()? {
        command.env("TZ", timezone);
    }
    if managed {
        command.env_remove("GIT_CONFIG_COUNT");
        for variable in ROUTING_ENVIRONMENT {
            command.env_remove(variable);
        }
    }
    if !allow_stdin {
        command.stdin(Stdio::null());
    }
    if allow_stdin && !managed && terminal_is_interactive() {
        let status = command
            .status()
            .with_context(|| format!("failed to execute Git in {}", directory.display()))?;
        return Ok(GitOutput {
            success: status.success(),
            code: status.code(),
            stdout: String::new(),
            stderr: String::new(),
        });
    }
    if allow_stdin && managed && io::stderr().is_terminal() {
        command.stderr(Stdio::inherit());
    }
    let output = command
        .output()
        .with_context(|| format!("failed to execute Git in {}", directory.display()))?;
    Ok(GitOutput {
        success: output.status.success(),
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn terminal_is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal()
}

pub fn run_os(
    directory: &Path,
    args: &[OsString],
    managed: bool,
    allow_stdin: bool,
) -> Result<GitOutput> {
    run(directory, args, managed, allow_stdin)
}

pub fn clone_repository(url: &str, target: &Path, options: CloneOptions<'_>) -> Result<()> {
    let mut command = Command::new("git");
    command.arg("clone").args(["--origin", options.remote_name]);
    if let Some(branch) = options.branch {
        command.args(["--branch", branch]);
    }
    if let Some(depth) = options.depth {
        command.args(["--depth", &depth.to_string()]);
        command.arg("--no-local");
    }
    if options.single_branch {
        command.arg("--single-branch");
    }
    command.arg("--").arg(url).arg(target);
    command.env_remove("GIT_CONFIG_COUNT");
    for variable in ROUTING_ENVIRONMENT {
        command.env_remove(variable);
    }
    if !options.allow_stdin {
        command.stdin(Stdio::null());
    }

    if let Some(progress) = options.progress {
        command.stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .with_context(|| format!("failed to execute Git clone for {url}"))?;
        let stderr = child
            .stderr
            .take()
            .context("failed to capture Git clone progress")?;
        let mut message = Vec::new();
        for chunk in BufReader::new(stderr).split(b'\r') {
            let chunk = chunk.context("failed to read Git clone progress")?;
            message.extend_from_slice(&chunk);
            message.push(b'\n');
            if let Some((received, total)) = clone_progress_counts(&chunk) {
                progress(received, total);
            }
        }
        let status = child.wait().context("failed to wait for Git clone")?;
        if !status.success() {
            bail!(
                "Git clone failed with {status}: {}",
                String::from_utf8_lossy(&message).trim()
            );
        }
        return Ok(());
    }

    if options.allow_stdin && terminal_is_interactive() {
        let status = command
            .status()
            .with_context(|| format!("failed to execute Git clone for {url}"))?;
        if !status.success() {
            bail!("Git clone failed with {status}");
        }
        return Ok(());
    }

    let output = command
        .output()
        .with_context(|| format!("failed to execute Git clone for {url}"))?;
    if !output.status.success() {
        bail!(
            "Git clone failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn clone_progress_counts(message: &[u8]) -> Option<(usize, usize)> {
    let message = String::from_utf8_lossy(message);
    let counters = message.rsplit_once('(')?.1.strip_suffix(')')?;
    let (received, total) = counters.split_once('/')?;
    Some((received.trim().parse().ok()?, total.trim().parse().ok()?))
}

pub fn fetch_all(path: &Path, allow_stdin: bool) -> Result<GitOutput> {
    run(path, ["fetch", "--all", "--prune"], true, allow_stdin)
}

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

pub fn create_and_checkout_branch(
    path: &Path,
    branch_name: &str,
    start_point: Option<&str>,
    remote_name: Option<&str>,
) -> Result<()> {
    let full_name = format!("refs/heads/{branch_name}");
    if !git2::Reference::is_valid_name(&full_name) {
        bail!("invalid branch name: {branch_name}");
    }
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

    match checkout_target_from_remotes(repository, start)? {
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

fn checkout_target_from_remotes(repository: &Repository, branch: &str) -> Result<CheckoutTarget> {
    let mut matches = Vec::new();
    for branch_result in repository
        .branches(Some(BranchType::Remote))
        .context("failed to enumerate remote branches")?
    {
        let (candidate, _) = branch_result.context("failed to inspect remote branch")?;
        let Some(name) = candidate
            .name()
            .context("remote branch name is not UTF-8")?
        else {
            continue;
        };
        let Some((_, logical_name)) = name.split_once('/') else {
            continue;
        };
        if logical_name == branch {
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
    for name in names.iter().flatten() {
        let remote = repository
            .find_remote(name)
            .with_context(|| format!("failed to read remote {name}"))?;
        let Some(url) = remote.url() else {
            continue;
        };
        remotes.push(RemoteRecord {
            name: name.to_owned(),
            fetch_url: portable_remote_url(url),
            push_url: remote.pushurl().map(portable_remote_url),
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

pub fn is_repository(path: &Path) -> bool {
    Repository::open(path)
        .map(|repository| !repository.is_bare())
        .unwrap_or(false)
}

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
        head.shorthand().map(str::to_owned)
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

fn unavailable_runtime(state: RepositoryRuntimeState) -> RepositoryRuntimeInfo {
    RepositoryRuntimeInfo {
        state,
        current_branch: None,
        head: None,
        local_branches: None,
        remote_branches: None,
    }
}

fn branch_count(repository: &Repository, branch_type: BranchType) -> Option<usize> {
    let branches = repository.branches(Some(branch_type)).ok()?;
    let mut count = 0;
    for result in branches {
        let (branch, actual_type) = result.ok()?;
        if actual_type == BranchType::Remote && branch.get().symbolic_target().is_some() {
            continue;
        }
        count += 1;
    }
    Some(count)
}

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
        .and_then(|head| head.name().map(str::to_owned));
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
        if actual_type == BranchType::Remote && reference.symbolic_target().is_some() {
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
                    .is_some_and(|name| Some(name) == current_reference),
            commit_short: commit_id.chars().take(8).collect(),
            commit: commit_id,
            commit_time: commit.time().seconds(),
        });
    }
    Ok(())
}

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
    let Some(branch_name) = head.shorthand() else {
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

pub fn upstream_push_target(path: &Path, branch: &str) -> Result<Option<(String, String)>> {
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    let config = repository
        .config()
        .context("failed to read Git configuration")?;
    let remote_key = format!("branch.{branch}.remote");
    let merge_key = format!("branch.{branch}.merge");
    let remote = match config.get_string(&remote_key) {
        Ok(remote) => remote,
        Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if remote == "." {
        bail!("current branch upstream is not a remote branch");
    }
    let merge_ref = config
        .get_string(&merge_key)
        .with_context(|| format!("current branch has no configured upstream ref: {branch}"))?;
    if !merge_ref.starts_with("refs/heads/") {
        bail!("current branch upstream is not a remote branch: {merge_ref}");
    }
    Ok(Some((remote, merge_ref)))
}

pub fn checkout_target(path: &Path, branch: &str, remote: Option<&str>) -> Result<CheckoutTarget> {
    let full_name = format!("refs/heads/{branch}");
    if !git2::Reference::is_valid_name(&full_name) {
        bail!("invalid branch name: {branch}");
    }

    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    if repository.find_branch(branch, BranchType::Local).is_ok() {
        return Ok(CheckoutTarget::Local);
    }

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

fn remote_url(path: &Path, remote_name: &str) -> Result<Option<String>> {
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    match repository.find_remote(remote_name) {
        Ok(remote) => Ok(remote.url().map(portable_remote_url)),
        Err(error) if error.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn verify_declared_remotes(path: &Path, repository: &RepositoryRecord) -> Result<()> {
    for remote in &repository.remotes {
        match remote_url(path, &remote.name)? {
            Some(actual) if actual == remote.fetch_url => {}
            Some(actual) => bail!(
                "remote {} URL mismatch: expected {}, found {}",
                remote.name,
                remote.fetch_url,
                actual
            ),
            None => bail!("declared remote {} is missing", remote.name),
        }
    }
    Ok(())
}

pub fn configure_declared_remotes(
    path: &Path,
    repository: &RepositoryRecord,
    _allow_stdin: bool,
) -> Result<()> {
    let git_repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    for remote in &repository.remotes {
        match git_repository.find_remote(&remote.name) {
            Ok(_) => {}
            Err(error) if error.code() == git2::ErrorCode::NotFound => {
                git_repository
                    .remote(&remote.name, &remote.fetch_url)
                    .with_context(|| format!("failed to add remote {}", remote.name))?;
            }
            Err(error) => return Err(error.into()),
        }
    }

    let mut config = git_repository
        .config()
        .context("failed to open repository config")?;
    for remote in &repository.remotes {
        let refspec = format!("+refs/heads/*:refs/remotes/{}/*", remote.name);
        config
            .set_str(&format!("remote.{}.fetch", remote.name), &refspec)
            .with_context(|| format!("failed to configure fetch refspec for {}", remote.name))?;

        let push_url = remote
            .push_url
            .as_deref()
            .filter(|push_url| *push_url != remote.fetch_url);
        let has_push_url = git_repository
            .find_remote(&remote.name)
            .with_context(|| format!("failed to read remote {}", remote.name))?
            .pushurl()
            .is_some();
        if push_url.is_some() || has_push_url {
            git_repository
                .remote_set_pushurl(&remote.name, push_url)
                .with_context(|| format!("failed to configure push URL for {}", remote.name))?;
        }
    }
    Ok(())
}

pub fn discover(root: &Path, max_depth: usize) -> Result<Vec<PathBuf>> {
    if max_depth == 0 {
        bail!("scan depth must be at least 1");
    }
    let mut repositories = Vec::new();
    discover_below(root, root, 0, max_depth, &mut repositories)?;
    repositories.sort();
    repositories.dedup();
    Ok(repositories)
}

fn discover_below(
    root: &Path,
    directory: &Path,
    depth: usize,
    max_depth: usize,
    repositories: &mut Vec<PathBuf>,
) -> Result<()> {
    if depth >= max_depth {
        return Ok(());
    }
    let mut children = fs::read_dir(directory)
        .with_context(|| format!("failed to read directory {}", directory.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let file_type = child.file_type()?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let path = child.path();
        if path == root.join("target") || path.file_name().is_some_and(|name| name == ".git") {
            continue;
        }
        if is_repository(&path) {
            repositories.push(path);
        } else {
            discover_below(root, &path, depth + 1, max_depth, repositories)?;
        }
    }
    Ok(())
}

fn detect_default_branch(repository: &Repository, primary_remote: &str) -> String {
    let remote_head = format!("refs/remotes/{primary_remote}/HEAD");
    if let Ok(reference) = repository.find_reference(&remote_head)
        && let Some(target) = reference.symbolic_target()
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
        && let Some(branch) = head.shorthand()
    {
        return branch.to_owned();
    }
    "main".to_owned()
}

fn portable_remote_url(raw: &str) -> String {
    let Ok(mut parsed) = Url::parse(raw) else {
        return raw.to_owned();
    };
    if matches!(parsed.scheme(), "http" | "https")
        && (!parsed.username().is_empty() || parsed.password().is_some())
    {
        let _ = parsed.set_username("");
        let _ = parsed.set_password(None);
    }
    parsed.to_string()
}

pub fn display_remote_url(raw: &str) -> String {
    portable_remote_url(raw)
}

#[cfg(test)]
mod tests {
    use crate::model::{RemoteRecord, RepositoryRecord, now};

    use super::{configure_declared_remotes, display_remote_url};

    #[test]
    fn display_remote_url_removes_http_credentials() {
        assert_eq!(
            display_remote_url("https://user:secret@example.com/team/repository.git"),
            "https://example.com/team/repository.git"
        );
        assert_eq!(
            display_remote_url("git@example.com:team/repository.git"),
            "git@example.com:team/repository.git"
        );
    }

    #[test]
    fn declared_push_url_replaces_and_clears_local_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let repository = git2::Repository::init(directory.path()).unwrap();
        repository
            .remote("origin", "https://example.com/fetch.git")
            .unwrap();
        repository
            .remote_set_pushurl("origin", Some("https://example.com/stale.git"))
            .unwrap();

        let timestamp = now();
        let mut record = RepositoryRecord {
            name: "example".to_owned(),
            directory: "example".to_owned(),
            default_branch: "main".to_owned(),
            primary_remote: "origin".to_owned(),
            remotes: vec![RemoteRecord {
                name: "origin".to_owned(),
                fetch_url: "https://example.com/fetch.git".to_owned(),
                push_url: None,
            }],
            created_at: timestamp,
            synced_at: None,
        };

        configure_declared_remotes(directory.path(), &record, false).unwrap();
        assert!(
            repository
                .find_remote("origin")
                .unwrap()
                .pushurl()
                .is_none()
        );

        record.remotes[0].push_url = Some("https://example.com/push.git".to_owned());
        configure_declared_remotes(directory.path(), &record, false).unwrap();
        assert_eq!(
            repository.find_remote("origin").unwrap().pushurl(),
            Some("https://example.com/push.git")
        );
    }
}
