use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::io::{self, IsTerminal, Read};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use super::types::{GitExecutionOptions, GitOutput};
use crate::error::{ErrorCode, classified};

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

/// Maximum retained bytes for each captured child stream.
pub(super) const MAX_CAPTURED_OUTPUT_BYTES: usize = 1024 * 1024;
pub(super) const OUTPUT_TRUNCATION_MARKER: &[u8] = b"\n[batch-git: output truncated]\n";

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
        isolate_repository_environment(&mut command);
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
        None => output_without_timeout(&mut command, !inherit_stderr, "Git command")
            .with_context(|| format!("failed to execute Git in {}", directory.display()))?,
    };
    Ok(GitOutput {
        success: output.status.success(),
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
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

/// 清除会把子进程重定向到调用方仓库的环境变量。
pub(super) fn isolate_repository_environment(command: &mut Command) {
    command.env_remove("GIT_CONFIG_COUNT");
    for variable in ROUTING_ENVIRONMENT {
        command.env_remove(variable);
    }
}

/// 只有三个标准流都连接终端时才安全进入完全交互模式。
pub(super) fn terminal_is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal()
}

/// 将自动化执行约束应用到一个即将启动的 Git 子进程。
pub(super) fn configure_execution_options(command: &mut Command, options: GitExecutionOptions) {
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

/// 在保留调用方标准流的情况下执行子进程，并在超时后终止直接子进程。
pub(super) fn status_with_timeout(
    command: &mut Command,
    timeout: Duration,
    operation: &str,
) -> Result<ExitStatus> {
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to execute {operation}"))?;
    let (status, timed_out) = wait_for_child_with_timeout(&mut child, timeout)?;
    if timed_out {
        return Err(classified(
            ErrorCode::Timeout,
            timeout_message(operation, timeout),
        ));
    }
    Ok(status)
}

/// 捕获子进程输出并在超时后终止子进程；读取线程避免大量 Git 输出造成管道死锁。
pub(super) fn output_with_timeout(
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
        return Err(classified(
            ErrorCode::Timeout,
            timeout_message(operation, timeout),
        ));
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

/// Capture a child without a deadline while applying the same bounded stream policy.
pub(super) fn output_without_timeout(
    command: &mut Command,
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
    let status = child
        .wait()
        .with_context(|| format!("failed to wait for {operation}"))?;
    let stdout = stdout
        .map(|reader| join_reader(reader, "failed to read Git stdout"))
        .transpose()?;
    let stderr = stderr
        .map(|reader| join_reader(reader, "failed to read Git stderr"))
        .transpose()?;
    Ok(Output {
        status,
        stdout: stdout.unwrap_or_default(),
        stderr: stderr.unwrap_or_default(),
    })
}

/// 在后台持续排空一个子进程输出流，避免子进程因填满 pipe 而无法退出。
fn spawn_output_reader<R: Read + Send + 'static>(reader: R) -> thread::JoinHandle<Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut reader = reader;
        let mut output = BoundedOutput::new(MAX_CAPTURED_OUTPUT_BYTES);
        let mut buffer = [0_u8; 8192];
        loop {
            let read = reader
                .read(&mut buffer)
                .context("failed to read Git child output")?;
            if read == 0 {
                break;
            }
            output.push(&buffer[..read]);
        }
        Ok(output.finish())
    })
}

/// Retain complete small output and a diagnostic head/tail window after truncation.
pub(super) struct BoundedOutput {
    limit: usize,
    head_limit: usize,
    tail_limit: usize,
    head: Vec<u8>,
    tail: VecDeque<u8>,
    truncated: bool,
}

impl BoundedOutput {
    pub(super) fn new(limit: usize) -> Self {
        let retained = limit.saturating_sub(OUTPUT_TRUNCATION_MARKER.len());
        let head_limit = retained / 2;
        Self {
            limit,
            head_limit,
            tail_limit: retained - head_limit,
            head: Vec::with_capacity(limit),
            tail: VecDeque::new(),
            truncated: false,
        }
    }

    pub(super) fn push(&mut self, bytes: &[u8]) {
        if !self.truncated && self.head.len().saturating_add(bytes.len()) <= self.limit {
            self.head.extend_from_slice(bytes);
            return;
        }
        if !self.truncated {
            self.truncated = true;
            let overflow = self.head.split_off(self.head_limit.min(self.head.len()));
            self.push_tail(&overflow);
        }
        self.push_tail(bytes);
    }

    fn push_tail(&mut self, bytes: &[u8]) {
        if self.tail_limit == 0 {
            return;
        }
        self.tail.extend(bytes);
        let excess = self.tail.len().saturating_sub(self.tail_limit);
        self.tail.drain(..excess);
    }

    pub(super) fn finish(mut self) -> Vec<u8> {
        if !self.truncated {
            return self.head;
        }
        self.head.extend_from_slice(OUTPUT_TRUNCATION_MARKER);
        self.head.extend(self.tail);
        debug_assert!(self.head.len() <= self.limit);
        self.head
    }
}

/// 将子进程输出读取线程的 panic 转换为普通错误，避免让 CLI 线程 panic。
pub(super) fn join_reader<T>(reader: thread::JoinHandle<Result<T>>, label: &str) -> Result<T> {
    reader
        .join()
        .map_err(|_| anyhow::anyhow!("{label}: reader thread panicked"))?
}

/// 轮询等待子进程；超时后 kill 并 wait，确保子进程不会成为僵尸进程。
pub(super) fn wait_for_child_with_timeout(
    child: &mut Child,
    timeout: Duration,
) -> Result<(ExitStatus, bool)> {
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
pub(super) fn timeout_message(operation: &str, timeout: Duration) -> String {
    format!("{operation} timed out after {} ms", timeout.as_millis())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::io::Cursor;
    use std::process::Command;
    use std::time::{Duration, Instant};

    use super::{
        MAX_CAPTURED_OUTPUT_BYTES, OUTPUT_TRUNCATION_MARKER, configure_execution_options,
        output_without_timeout, spawn_output_reader, wait_for_child_with_timeout,
    };
    use crate::git::types::GitExecutionOptions;

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

    #[test]
    fn child_output_is_drained_but_retained_within_the_limit() {
        let mut input = vec![b'a'; MAX_CAPTURED_OUTPUT_BYTES];
        input.extend(vec![b'b'; MAX_CAPTURED_OUTPUT_BYTES]);
        let output = spawn_output_reader(Cursor::new(input))
            .join()
            .unwrap()
            .unwrap();

        assert!(output.len() <= MAX_CAPTURED_OUTPUT_BYTES);
        assert!(output.starts_with(&vec![b'a'; 1024]));
        assert!(output.ends_with(&vec![b'b'; 1024]));
        assert!(
            output
                .windows(OUTPUT_TRUNCATION_MARKER.len())
                .any(|window| window == OUTPUT_TRUNCATION_MARKER)
        );
    }

    #[test]
    fn small_child_output_is_preserved_exactly() {
        let input = b"complete output\n".to_vec();
        let output = spawn_output_reader(Cursor::new(input.clone()))
            .join()
            .unwrap()
            .unwrap();
        assert_eq!(output, input);
    }

    #[cfg(unix)]
    #[test]
    fn large_child_pipe_output_completes_without_unbounded_retention() {
        let mut command = Command::new("sh");
        command.args(["-c", "yes x | head -c 2097152"]);
        let output = output_without_timeout(&mut command, true, "large output fixture").unwrap();

        assert!(output.status.success());
        assert!(output.stdout.len() <= MAX_CAPTURED_OUTPUT_BYTES);
        assert!(
            output
                .stdout
                .windows(OUTPUT_TRUNCATION_MARKER.len())
                .any(|window| window == OUTPUT_TRUNCATION_MARKER)
        );
    }
}
