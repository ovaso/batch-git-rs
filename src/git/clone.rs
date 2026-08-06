use std::io::Read;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;

use anyhow::{Context, Result, bail};

use super::execution::{
    BoundedOutput, MAX_CAPTURED_OUTPUT_BYTES, configure_execution_options,
    isolate_repository_environment, join_reader, output_with_timeout, output_without_timeout,
    status_with_timeout, terminal_is_interactive, timeout_message, wait_for_child_with_timeout,
};
use super::types::{CloneOptions, CloneProgress, GitExecutionOptions};
use crate::error::{ErrorCode, classified};

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
    isolate_repository_environment(&mut command);
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
        None => output_without_timeout(command, true, "Git clone")
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
                return Err(classified(
                    ErrorCode::Timeout,
                    timeout_message("Git clone", timeout),
                ));
            }
            let message = join_reader(reader, "failed to read Git clone progress");
            ensure_clone_succeeded(status, &message?, execution.non_interactive)
        }
    }
}

/// 收集 clone stderr 的进度行，并保留失败时可供人类诊断的文本。
fn read_clone_progress<R: Read>(reader: R, progress: &CloneProgress) -> Result<Vec<u8>> {
    const MAX_PROGRESS_LINE_BYTES: usize = 8192;

    let mut reader = reader;
    let mut message = BoundedOutput::new(MAX_CAPTURED_OUTPUT_BYTES);
    let mut progress_line = Vec::with_capacity(256);
    let mut progress_line_overflowed = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut buffer)
            .context("failed to read Git clone progress")?;
        if read == 0 {
            break;
        }
        let mut diagnostic = Vec::with_capacity(read);
        for &byte in &buffer[..read] {
            if byte == b'\r' {
                diagnostic.push(b'\n');
                if !progress_line_overflowed
                    && let Some((received, total)) = clone_progress_counts(&progress_line)
                {
                    progress(received, total);
                }
                progress_line.clear();
                progress_line_overflowed = false;
            } else {
                diagnostic.push(byte);
                if progress_line.len() < MAX_PROGRESS_LINE_BYTES {
                    progress_line.push(byte);
                } else {
                    progress_line_overflowed = true;
                }
            }
        }
        message.push(&diagnostic);
    }
    if !progress_line.is_empty()
        && !progress_line_overflowed
        && let Some((received, total)) = clone_progress_counts(&progress_line)
    {
        progress(received, total);
    }
    Ok(message.finish())
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

/// 从 Git stderr 进度行中提取“当前/总数”，无法识别时返回 None。
fn clone_progress_counts(message: &[u8]) -> Option<(usize, usize)> {
    let message = String::from_utf8_lossy(message);
    let counters = message.rsplit_once('(')?.1.strip_suffix(')')?;
    let (received, total) = counters.split_once('/')?;
    Some((received.trim().parse().ok()?, total.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::process::Command;
    use std::sync::Arc;
    use std::time::Duration;

    use super::{GitExecutionOptions, execute_clone_command, read_clone_progress};
    use crate::git::execution::{MAX_CAPTURED_OUTPUT_BYTES, OUTPUT_TRUNCATION_MARKER};

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
    fn clone_progress_diagnostics_are_bounded() {
        let input = vec![b'x'; MAX_CAPTURED_OUTPUT_BYTES * 2];
        let progress: super::CloneProgress = Arc::new(|_, _| {});
        let output = read_clone_progress(Cursor::new(input), &progress).unwrap();

        assert!(output.len() <= MAX_CAPTURED_OUTPUT_BYTES);
        assert!(
            output
                .windows(OUTPUT_TRUNCATION_MARKER.len())
                .any(|window| window == OUTPUT_TRUNCATION_MARKER)
        );
    }
}
