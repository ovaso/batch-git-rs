//! Git repository inspection through `git2` and compatibility-sensitive operations through Git.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use git2::build::CheckoutBuilder;
use git2::{BranchType, Repository, Status, StatusOptions};
use url::Url;

use crate::model::{RemoteRecord, RepositoryRecord};

/// 可能把子进程重定向到调用方仓库的 Git 环境变量。
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

/// 系统 Git 子进程的完整、可聚合执行结果。
#[derive(Debug)]
pub struct GitOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// 执行系统 Git 时可由调用方统一控制的交互与时间边界。
///
/// `non_interactive` 优先于 `allow_stdin`：启用后会关闭标准输入并禁止 Git 的终端
/// 凭据提示。`timeout` 仅约束本次 Git 子进程；超时后会终止并回收直接子进程。
#[derive(Debug, Clone, Copy, Default)]
pub struct GitExecutionOptions {
    pub allow_stdin: bool,
    pub non_interactive: bool,
    pub timeout: Option<Duration>,
}

impl GitExecutionOptions {
    /// 从旧调用点的单个 `allow_stdin` 参数构造兼容选项。
    #[allow(dead_code)] // Compatibility wrappers below remain useful to in-module callers.
    pub const fn legacy(allow_stdin: bool) -> Self {
        Self {
            allow_stdin,
            non_interactive: false,
            timeout: None,
        }
    }
}

/// 扫描仓库时写入清单的稳定 Git 元数据。
#[derive(Debug)]
pub struct RepositoryInfo {
    pub default_branch: String,
    pub primary_remote: String,
    pub remotes: Vec<RemoteRecord>,
}

/// 清单仓库在本地文件系统中的实时状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryRuntimeState {
    Available,
    Missing,
    NotGit,
    Bare,
    Error,
}

impl RepositoryRuntimeState {
    /// 返回适合表格和 JSON 输出的稳定标识。
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

/// 不访问网络即可取得的仓库运行时摘要。
#[derive(Debug)]
pub struct RepositoryRuntimeInfo {
    pub state: RepositoryRuntimeState,
    pub current_branch: Option<String>,
    pub head: Option<String>,
    pub local_branches: Option<usize>,
    pub remote_branches: Option<usize>,
}

/// 分支来自本地引用还是 remote-tracking 引用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchKind {
    Local,
    Remote,
}

impl BranchKind {
    /// 返回稳定的机器可读类别名。
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

/// 用于 find/list 输出的单个分支摘要。
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

/// checkout 名称解析后的确定目标或歧义状态。
#[derive(Debug)]
pub enum CheckoutTarget {
    Local,
    Remote(String),
    Missing,
    Ambiguous(Vec<String>),
}

/// 映射到系统 `git clone` 的受控选项。
pub struct CloneOptions<'a> {
    pub remote_name: &'a str,
    pub branch: Option<&'a str>,
    pub depth: Option<usize>,
    pub single_branch: bool,
    pub progress: Option<CloneProgress>,
    #[allow(dead_code)] // Read by the legacy clone_repository wrapper.
    pub allow_stdin: bool,
}

/// clone 进度回调，参数分别为已完成对象数和总对象数。
pub type CloneProgress = Arc<dyn Fn(usize, usize) + Send + Sync>;

/// 按 Git 状态类别聚合的工作树变更数量。
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
    /// 返回所有状态类别的总变更数。
    pub fn total(&self) -> usize {
        self.modified + self.added + self.deleted + self.renamed + self.untracked + self.conflicted
    }

    /// 生成类似 `M2 ?1` 的紧凑人类可读文本。
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

/// 当前分支相对 upstream 的提交关系。
#[derive(Debug)]
pub enum UpstreamSummary {
    UpToDate,
    Ahead(usize),
    Behind(usize),
    Diverged { ahead: usize, behind: usize },
    None,
}

impl UpstreamSummary {
    /// 生成人类可读的 ahead/behind 摘要。
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

/// `status` 命令所需的仓库级汇总。
#[derive(Debug)]
pub struct RepositoryStatusSummary {
    pub branch: String,
    pub changes: ChangeCounts,
    pub upstream: UpstreamSummary,
}

/// 在指定仓库运行系统 Git，并根据托管模式隔离调用方的路由环境变量。
#[allow(dead_code)] // Retained as an internal compatibility wrapper for the previous call shape.
pub fn run<I, S>(directory: &Path, args: I, managed: bool, allow_stdin: bool) -> Result<GitOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_with_options(
        directory,
        args,
        managed,
        GitExecutionOptions::legacy(allow_stdin),
    )
}

/// 在指定仓库运行系统 Git，并使用显式的自动化执行选项。
pub fn run_with_options<I, S>(
    directory: &Path,
    args: I,
    managed: bool,
    options: GitExecutionOptions,
) -> Result<GitOutput>
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
        // batch-git 自己构造的操作必须针对传入目录，不能继承外层 Git 的仓库定位。
        command.env_remove("GIT_CONFIG_COUNT");
        for variable in ROUTING_ENVIRONMENT {
            command.env_remove(variable);
        }
    }
    configure_execution_options(&mut command, options);

    if may_use_full_terminal(options, managed) {
        // 单任务交互透传直接继承终端，支持编辑器、密码提示和 rebase -i。
        let status = match options.timeout {
            Some(timeout) => status_with_timeout(&mut command, timeout, "Git command")?,
            None => command
                .status()
                .with_context(|| format!("failed to execute Git in {}", directory.display()))?,
        };
        return Ok(GitOutput {
            success: status.success(),
            code: status.code(),
            stdout: String::new(),
            stderr: String::new(),
        });
    }

    // 非交互/机器模式绝不继承子进程 stderr，避免它破坏调用方的结构化 stdout/stderr 边界。
    let inherit_stderr =
        options.allow_stdin && !options.non_interactive && managed && io::stderr().is_terminal();
    if inherit_stderr {
        command.stderr(Stdio::inherit());
    }
    let output = match options.timeout {
        Some(timeout) => {
            output_with_timeout(&mut command, timeout, !inherit_stderr, "Git command")?
        }
        None => command
            .output()
            .with_context(|| format!("failed to execute Git in {}", directory.display()))?,
    };
    Ok(GitOutput {
        success: output.status.success(),
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// 只有三个标准流都连接终端时才安全进入完全交互模式。
fn terminal_is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal()
}

/// 将自动化执行约束应用到一个即将启动的 Git 子进程。
fn configure_execution_options(command: &mut Command, options: GitExecutionOptions) {
    if options.non_interactive {
        // Git 的 credential helper 可能仍可工作，但绝不能等待终端输入。
        command.stdin(Stdio::null()).env("GIT_TERMINAL_PROMPT", "0");
    } else if !options.allow_stdin {
        command.stdin(Stdio::null());
    }
}

/// 是否允许沿用旧 API 的完全交互式终端透传行为。
fn may_use_full_terminal(options: GitExecutionOptions, managed: bool) -> bool {
    options.allow_stdin && !options.non_interactive && !managed && terminal_is_interactive()
}

/// 接受不可假定为 UTF-8 的原始参数并调用通用 Git 执行器。
#[allow(dead_code)] // Retained as an internal compatibility wrapper for the previous call shape.
pub fn run_os(
    directory: &Path,
    args: &[OsString],
    managed: bool,
    allow_stdin: bool,
) -> Result<GitOutput> {
    run_os_with_options(
        directory,
        args,
        managed,
        GitExecutionOptions::legacy(allow_stdin),
    )
}

/// 接受原始参数并使用显式的自动化执行选项调用 Git。
pub fn run_os_with_options(
    directory: &Path,
    args: &[OsString],
    managed: bool,
    options: GitExecutionOptions,
) -> Result<GitOutput> {
    run_with_options(directory, args, managed, options)
}

/// 使用系统 Git 克隆仓库，以继承用户的认证、代理和 credential helper 配置。
#[allow(dead_code)] // Retained as an internal compatibility wrapper for the previous call shape.
pub fn clone_repository(url: &str, target: &Path, options: CloneOptions<'_>) -> Result<()> {
    let execution = GitExecutionOptions::legacy(options.allow_stdin);
    clone_repository_with_options(url, target, options, execution)
}

/// 使用系统 Git 克隆仓库，并使用显式的自动化执行选项控制交互与超时。
///
/// `CloneOptions::allow_stdin` 仅为旧 API 保留；调用此函数时以 `execution.allow_stdin`
/// 为准。
pub fn clone_repository_with_options(
    url: &str,
    target: &Path,
    options: CloneOptions<'_>,
    execution: GitExecutionOptions,
) -> Result<()> {
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
    execute_clone_command(&mut command, url, options.progress, execution)
}

/// 在已构造的 clone 命令上应用执行策略；保留为独立函数以便测试超时与脱敏行为。
fn execute_clone_command(
    command: &mut Command,
    url: &str,
    progress: Option<CloneProgress>,
    execution: GitExecutionOptions,
) -> Result<()> {
    configure_execution_options(command, execution);

    if let Some(progress) = progress {
        command.stdout(Stdio::null()).stderr(Stdio::piped());
        return run_clone_with_progress(command, url, progress, execution);
    }

    if execution.allow_stdin && !execution.non_interactive && terminal_is_interactive() {
        let status = match execution.timeout {
            Some(timeout) => status_with_timeout(command, timeout, "Git clone")?,
            None => command
                .status()
                .with_context(|| format!("failed to execute Git clone for {url}"))?,
        };
        return ensure_clone_succeeded(status, &[], execution.non_interactive);
    }

    let output = match execution.timeout {
        Some(timeout) => output_with_timeout(command, timeout, true, "Git clone")?,
        None => command
            .output()
            .with_context(|| format!("failed to execute Git clone for {url}"))?,
    };
    ensure_clone_succeeded(output.status, &output.stderr, execution.non_interactive)
}

/// 运行带进度回调的 clone；超时时在等待主线程终止子进程后回收输出读取线程。
fn run_clone_with_progress(
    command: &mut Command,
    url: &str,
    progress: CloneProgress,
    execution: GitExecutionOptions,
) -> Result<()> {
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to execute Git clone for {url}"))?;
    let stderr = child
        .stderr
        .take()
        .context("failed to capture Git clone progress")?;

    match execution.timeout {
        None => {
            let message = read_clone_progress(stderr, &progress)?;
            let status = child.wait().context("failed to wait for Git clone")?;
            ensure_clone_succeeded(status, &message, execution.non_interactive)
        }
        Some(timeout) => {
            let reader = thread::spawn(move || read_clone_progress(stderr, &progress));
            let (status, timed_out) = wait_for_child_with_timeout(&mut child, timeout)?;
            if timed_out {
                // A Git transport helper can outlive the direct child and retain its stderr
                // pipe. Do not wait for that helper through the reader thread: the caller's
                // timeout must bound this invocation even though it cannot kill descendants on
                // every platform.
                drop(reader);
                bail!("{}", timeout_message("Git clone", timeout));
            }
            let message = join_reader(reader, "failed to read Git clone progress");
            ensure_clone_succeeded(status, &message?, execution.non_interactive)
        }
    }
}

/// 收集 clone stderr 的进度行，并保留失败时可供人类诊断的文本。
fn read_clone_progress<R: Read>(reader: R, progress: &CloneProgress) -> Result<Vec<u8>> {
    let mut message = Vec::new();
    for chunk in BufReader::new(reader).split(b'\r') {
        let chunk = chunk.context("failed to read Git clone progress")?;
        message.extend_from_slice(&chunk);
        message.push(b'\n');
        if let Some((received, total)) = clone_progress_counts(&chunk) {
            progress(received, total);
        }
    }
    Ok(message)
}

/// 将 clone 的失败文本限制在人类交互模式，避免机器模式转发子进程输出。
fn ensure_clone_succeeded(status: ExitStatus, stderr: &[u8], non_interactive: bool) -> Result<()> {
    if status.success() {
        return Ok(());
    }
    if non_interactive || stderr.is_empty() {
        bail!("Git clone failed with {status}");
    }
    bail!(
        "Git clone failed with {status}: {}",
        String::from_utf8_lossy(stderr).trim()
    )
}

/// 在保留调用方标准流的情况下执行子进程，并在超时后终止直接子进程。
fn status_with_timeout(
    command: &mut Command,
    timeout: Duration,
    operation: &str,
) -> Result<ExitStatus> {
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to execute {operation}"))?;
    let (status, timed_out) = wait_for_child_with_timeout(&mut child, timeout)?;
    if timed_out {
        bail!("{}", timeout_message(operation, timeout));
    }
    Ok(status)
}

/// 捕获子进程输出并在超时后终止子进程；读取线程避免大量 Git 输出造成管道死锁。
fn output_with_timeout(
    command: &mut Command,
    timeout: Duration,
    capture_stderr: bool,
    operation: &str,
) -> Result<Output> {
    command.stdout(Stdio::piped());
    if capture_stderr {
        command.stderr(Stdio::piped());
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to execute {operation}"))?;
    let stdout = child.stdout.take().map(spawn_output_reader);
    let stderr = child.stderr.take().map(spawn_output_reader);
    let (status, timed_out) = wait_for_child_with_timeout(&mut child, timeout)?;
    if timed_out {
        // Child helpers may inherit either pipe after the direct Git process has been reaped.
        // Waiting for the drain threads would turn a per-child timeout into an unbounded wait.
        // Dropping their JoinHandles detaches them; they retain no protocol output and will end
        // when the inherited pipe closes.
        drop(stdout);
        drop(stderr);
        bail!("{}", timeout_message(operation, timeout));
    }
    let stdout = stdout
        .map(|reader| join_reader(reader, "failed to read Git stdout"))
        .transpose();
    let stderr = stderr
        .map(|reader| join_reader(reader, "failed to read Git stderr"))
        .transpose();
    Ok(Output {
        status,
        stdout: stdout?.unwrap_or_default(),
        stderr: stderr?.unwrap_or_default(),
    })
}

/// 在后台持续排空一个子进程输出流，避免子进程因填满 pipe 而无法退出。
fn spawn_output_reader<R: Read + Send + 'static>(reader: R) -> thread::JoinHandle<Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut reader = reader;
        let mut output = Vec::new();
        reader
            .read_to_end(&mut output)
            .context("failed to read Git child output")?;
        Ok(output)
    })
}

/// 将子进程输出读取线程的 panic 转换为普通错误，避免让 CLI 线程 panic。
fn join_reader<T>(reader: thread::JoinHandle<Result<T>>, label: &str) -> Result<T> {
    reader
        .join()
        .map_err(|_| anyhow::anyhow!("{label}: reader thread panicked"))?
}

/// 轮询等待子进程；超时后 kill 并 wait，确保子进程不会成为僵尸进程。
fn wait_for_child_with_timeout(child: &mut Child, timeout: Duration) -> Result<(ExitStatus, bool)> {
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .context("failed to poll Git child process")?
        {
            return Ok((status, false));
        }
        if started.elapsed() >= timeout {
            // 进程可能刚好在 try_wait 和 kill 之间退出；此时仍可通过 wait 回收状态。
            if let Err(error) = child.kill()
                && error.kind() != io::ErrorKind::InvalidInput
            {
                return Err(error).context("failed to terminate timed out Git child process");
            }
            let status = child
                .wait()
                .context("failed to wait for timed out Git child process")?;
            return Ok((status, true));
        }
        let remaining = timeout.checked_sub(started.elapsed()).unwrap_or_default();
        thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

/// 生成不含子进程输出的稳定超时错误文本。
fn timeout_message(operation: &str, timeout: Duration) -> String {
    format!("{operation} timed out after {} ms", timeout.as_millis())
}

/// 从 Git stderr 进度行中提取“当前/总数”，无法识别时返回 None。
fn clone_progress_counts(message: &[u8]) -> Option<(usize, usize)> {
    let message = String::from_utf8_lossy(message);
    let counters = message.rsplit_once('(')?.1.strip_suffix(')')?;
    let (received, total) = counters.split_once('/')?;
    Some((received.trim().parse().ok()?, total.trim().parse().ok()?))
}

/// fetch 所有远端并清理已删除的 remote-tracking 引用。
#[allow(dead_code)] // Retained as an internal compatibility wrapper for the previous call shape.
pub fn fetch_all(path: &Path, allow_stdin: bool) -> Result<GitOutput> {
    fetch_all_with_options(path, GitExecutionOptions::legacy(allow_stdin))
}

/// fetch 所有远端并使用显式的自动化执行选项。
pub fn fetch_all_with_options(path: &Path, options: GitExecutionOptions) -> Result<GitOutput> {
    run_with_options(path, ["fetch", "--all", "--prune"], true, options)
}

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

/// 搜索所有远端的同名分支，并保留歧义供调用方提示用户。
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

/// 解析当前分支配置的 push 远端和目标引用。
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

/// 按“本地优先、显式远端其次、全远端搜索最后”的规则解析 checkout。
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

/// 读取远端 fetch URL，并移除 HTTP(S) 凭据后返回。
fn remote_url(path: &Path, remote_name: &str) -> Result<Option<String>> {
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    match repository.find_remote(remote_name) {
        Ok(remote) => Ok(remote.url().ok().map(portable_remote_url)),
        Err(error) if error.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// 只验证本地远端配置是否与清单一致，不进行修改。
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

/// 让本地 remote 配置收敛到清单声明，包括清除遗留 push URL。
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
            .ok()
            .flatten()
            .is_some();
        if push_url.is_some() || has_push_url {
            git_repository
                .remote_set_pushurl(&remote.name, push_url)
                .with_context(|| format!("failed to configure push URL for {}", remote.name))?;
        }
    }
    Ok(())
}

/// 在给定深度内发现 Git 工作树，并返回稳定排序的路径。
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

/// 深度优先扫描目录；发现仓库后不再进入其内部继续搜索。
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

/// 将本地路径尽量转换为可复制的绝对 file URL 或规范文本。
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

/// 移除 HTTP(S) URL 中可能包含的用户名、密码或 token。
pub fn display_remote_url(raw: &str) -> String {
    portable_remote_url(raw)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::process::Command;
    use std::time::{Duration, Instant};

    use crate::model::{RemoteRecord, RepositoryRecord, now};

    use super::{
        GitExecutionOptions, configure_declared_remotes, configure_execution_options,
        display_remote_url, execute_clone_command, wait_for_child_with_timeout,
    };

    #[test]
    fn non_interactive_execution_disables_terminal_prompts() {
        let mut command = Command::new("git");
        configure_execution_options(
            &mut command,
            GitExecutionOptions {
                allow_stdin: true,
                non_interactive: true,
                timeout: None,
            },
        );
        let prompt = command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("GIT_TERMINAL_PROMPT"))
            .and_then(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()));
        assert_eq!(prompt.as_deref(), Some("0"));
    }

    #[cfg(unix)]
    #[test]
    fn timeout_terminates_and_reaps_child() {
        let mut child = Command::new("sh").args(["-c", "sleep 2"]).spawn().unwrap();
        let started = Instant::now();
        let (_, timed_out) =
            wait_for_child_with_timeout(&mut child, Duration::from_millis(20)).unwrap();
        assert!(timed_out);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn non_interactive_clone_timeout_does_not_expose_child_output() {
        let mut command = Command::new("sh");
        command.args(["-c", "echo machine-secret >&2; sleep 2"]);
        let error = execute_clone_command(
            &mut command,
            "test://clone",
            None,
            GitExecutionOptions {
                allow_stdin: false,
                non_interactive: true,
                timeout: Some(Duration::from_millis(20)),
            },
        )
        .unwrap_err();
        let rendered = format!("{error:#}");
        assert!(rendered.contains("timed out"));
        assert!(!rendered.contains("machine-secret"));
    }

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
                .unwrap()
                .is_none()
        );

        record.remotes[0].push_url = Some("https://example.com/push.git".to_owned());
        configure_declared_remotes(directory.path(), &record, false).unwrap();
        assert_eq!(
            repository.find_remote("origin").unwrap().pushurl().unwrap(),
            Some("https://example.com/push.git")
        );
    }
}
