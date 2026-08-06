//! Native scheduler file activation and system command boundaries.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use tempfile::NamedTempFile;

use super::{NativeArtifact, NativePlatform, RegistrationState};

/// 原子写入所有定义后激活任务；失败时尽量恢复旧文件。
pub(super) fn write_and_activate(
    artifact: &NativeArtifact,
    updating: bool,
    quiet: bool,
) -> Result<()> {
    if let Some(log_directory) = &artifact.log_directory {
        fs::create_dir_all(log_directory).with_context(|| {
            format!(
                "failed to create schedule log directory {}",
                log_directory.display()
            )
        })?;
    }
    let backups = artifact
        .files
        .iter()
        .map(|file| fs::read(&file.path).ok())
        .collect::<Vec<_>>();
    for file in &artifact.files {
        atomic_write(&file.path, file.content.as_bytes())?;
    }
    if let Err(error) = activate(artifact, updating, quiet) {
        for (file, backup) in artifact.files.iter().zip(backups) {
            match backup {
                Some(content) => {
                    let _ = atomic_write(&file.path, &content);
                }
                None => {
                    let _ = fs::remove_file(&file.path);
                }
            }
        }
        let _ = reactivate_restored_files(artifact, quiet);
        return Err(error);
    }
    Ok(())
}

/// 调用目标平台命令加载或更新用户级任务。
fn activate(artifact: &NativeArtifact, updating: bool, quiet: bool) -> Result<()> {
    match artifact.platform {
        NativePlatform::Launchd => {
            let domain = launchd_domain()?;
            if updating {
                let target = format!("{domain}/{}", artifact.task_id);
                let mut command = Command::new("launchctl");
                command.args(["bootout", &target]);
                suppress_command_output(&mut command, quiet);
                let _ = command.status();
            }
            let mut command = Command::new("launchctl");
            command
                .args(["bootstrap", &domain])
                .arg(&artifact.files[0].path);
            suppress_command_output(&mut command, quiet);
            let status = command.status().context("failed to execute launchctl")?;
            if !status.success() {
                bail!("launchctl bootstrap failed with {status}");
            }
        }
        NativePlatform::Systemd => {
            run_systemctl(["daemon-reload"], quiet)?;
            let timer = format!("{}.timer", artifact.task_id);
            if updating {
                run_systemctl(["restart", timer.as_str()], quiet)?;
                run_systemctl(["enable", timer.as_str()], quiet)?;
            } else {
                run_systemctl(["enable", "--now", timer.as_str()], quiet)?;
            }
        }
        NativePlatform::Windows => {
            let mut command = Command::new("schtasks.exe");
            command
                .args(["/Create", "/TN", &artifact.task_id, "/XML"])
                .arg(&artifact.files[0].path)
                .arg("/F");
            suppress_command_output(&mut command, quiet);
            let status = command.status().context("failed to execute schtasks.exe")?;
            if !status.success() {
                bail!("schtasks.exe /Create failed with {status}");
            }
        }
    }
    Ok(())
}

/// 根据注册摘要调用平台命令停用任务。
pub(super) fn deactivate(state: &RegistrationState, quiet: bool) -> Result<()> {
    match state.platform.as_str() {
        "launchd" => {
            let domain = launchd_domain()?;
            let target = format!("{domain}/{}", state.task_id);
            let mut command = Command::new("launchctl");
            command.args(["bootout", &target]);
            suppress_command_output(&mut command, quiet);
            let _ = command.status();
        }
        "systemd" => {
            let timer = format!("{}.timer", state.task_id);
            let mut command = Command::new("systemctl");
            command.args(["--user", "disable", "--now", timer.as_str()]);
            suppress_command_output(&mut command, quiet);
            let _ = command.status();
        }
        "windows" => {
            let mut command = Command::new("schtasks.exe");
            command.args(["/Delete", "/TN", &state.task_id, "/F"]);
            suppress_command_output(&mut command, quiet);
            let _ = command.status();
        }
        platform => bail!("unsupported registered schedule platform: {platform}"),
    }
    Ok(())
}

/// 更新失败并恢复旧文件后，尽力重新加载旧任务。
fn reactivate_restored_files(artifact: &NativeArtifact, quiet: bool) -> Result<()> {
    match artifact.platform {
        NativePlatform::Launchd => {
            if artifact.files[0].path.is_file() {
                let domain = launchd_domain()?;
                let mut command = Command::new("launchctl");
                command
                    .args(["bootstrap", &domain])
                    .arg(&artifact.files[0].path);
                suppress_command_output(&mut command, quiet);
                let status = command
                    .status()
                    .context("failed to restore previous launchd task")?;
                if !status.success() {
                    bail!("failed to restore previous launchd task: {status}");
                }
            }
        }
        NativePlatform::Systemd => {
            run_systemctl(["daemon-reload"], quiet)?;
            let timer = format!("{}.timer", artifact.task_id);
            let _ = run_systemctl(["restart", timer.as_str()], quiet);
        }
        NativePlatform::Windows => {
            if artifact.files[0].path.is_file() {
                let mut command = Command::new("schtasks.exe");
                command
                    .args(["/Create", "/TN", &artifact.task_id, "/XML"])
                    .arg(&artifact.files[0].path)
                    .arg("/F");
                suppress_command_output(&mut command, quiet);
                let status = command
                    .status()
                    .context("failed to restore previous Windows scheduled task")?;
                if !status.success() {
                    bail!("failed to restore previous Windows scheduled task: {status}");
                }
            }
        }
    }
    Ok(())
}

/// 执行 systemctl --user 并统一补充错误上下文。
fn run_systemctl<const N: usize>(arguments: [&str; N], quiet: bool) -> Result<()> {
    let mut command = Command::new("systemctl");
    command.arg("--user").args(arguments);
    suppress_command_output(&mut command, quiet);
    let status = command.status().context("failed to execute systemctl")?;
    if !status.success() {
        bail!("systemctl failed with {status}");
    }
    Ok(())
}

/// 在机器输出模式中屏蔽原生调度器命令的杂项诊断。
fn suppress_command_output(command: &mut Command, quiet: bool) {
    if quiet {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
}

/// 查询目标平台是否已经存在同 ID 的原生任务。
pub(super) fn native_task_exists(platform: NativePlatform, task_id: &str) -> Result<bool> {
    if platform != NativePlatform::Windows {
        return Ok(false);
    }
    #[cfg(target_os = "windows")]
    {
        let status = Command::new("schtasks.exe")
            .args(["/Query", "/TN", task_id])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("failed to execute schtasks.exe")?;
        Ok(status.success())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = task_id;
        Ok(false)
    }
}

/// 构造当前用户 launchd GUI domain，如 `gui/501`。
pub(super) fn launchd_domain() -> Result<String> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("failed to determine user id")?;
    if !output.status.success() {
        bail!("id -u failed with {}", output.status);
    }
    Ok(format!("gui/{}", String::from_utf8(output.stdout)?.trim()))
}

/// 在目标目录写临时文件并原子替换单个原生定义。
pub(super) fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    temporary.write_all(content)?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .ok();
    Ok(())
}

/// 只删除注册摘要中记录且通过安全校验的原生文件。
pub(super) fn remove_registered_files(state: &RegistrationState, quiet: bool) -> Result<()> {
    for file in &state.files {
        let path = PathBuf::from(file);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
    }
    if state.platform == "systemd" {
        run_systemctl(["daemon-reload"], quiet)?;
    }
    Ok(())
}
