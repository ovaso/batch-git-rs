//! Native scheduler artifact assembly and platform routing.

mod launchd;
mod systemd;
mod windows;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::json;

use super::{NativeArtifact, NativePlatform};
use crate::model::ScheduleRecord;
use crate::schedule::state::schedule_log_directory;
use crate::settings;

#[cfg(test)]
pub(super) use launchd::launchd_cron_trigger;
#[cfg(test)]
pub(super) use systemd::{systemd_cron_trigger, systemd_quote};
#[cfg(test)]
pub(super) use windows::{windows_argument, windows_repetition_interval};

struct ArtifactContext<'a> {
    root: &'a Path,
    schedule: &'a ScheduleRecord,
    executable: PathBuf,
    task_id: String,
    log_enabled: bool,
    log_directory: Option<PathBuf>,
    stdout: Option<PathBuf>,
    stderr: Option<PathBuf>,
    timezone: Option<&'a str>,
}

/// 使用当前日志和时区配置生成原生任务产物。
pub(super) fn build_artifact(
    root: &Path,
    schedule: &ScheduleRecord,
    platform: NativePlatform,
) -> Result<NativeArtifact> {
    build_artifact_with_options(
        root,
        schedule,
        platform,
        settings::schedule_log_enabled()?,
        settings::schedule_timezone()?.as_deref(),
    )
}

/// 使用显式日志开关生成产物，供 status 重建期望定义。
pub(super) fn build_artifact_with_logging(
    root: &Path,
    schedule: &ScheduleRecord,
    platform: NativePlatform,
    log_enabled: bool,
) -> Result<NativeArtifact> {
    build_artifact_with_options(
        root,
        schedule,
        platform,
        log_enabled,
        settings::schedule_timezone()?.as_deref(),
    )
}

/// 将统一 schedule 模型完整翻译为指定平台配置。
pub(super) fn build_artifact_with_options(
    root: &Path,
    schedule: &ScheduleRecord,
    platform: NativePlatform,
    log_enabled: bool,
    timezone: Option<&str>,
) -> Result<NativeArtifact> {
    let executable = env::current_exe()
        .context("failed to locate batch-git executable")?
        .canonicalize()
        .context("failed to canonicalize batch-git executable")?;
    let task_id = task_id(root, &schedule.name);
    let log_directory = log_enabled
        .then(|| schedule_log_directory(root, &schedule.name))
        .transpose()?;
    let stdout = log_directory.as_ref().map(|path| path.join("stdout.log"));
    let stderr = log_directory.as_ref().map(|path| path.join("stderr.log"));
    let context = ArtifactContext {
        root,
        schedule,
        executable,
        task_id,
        log_enabled,
        log_directory,
        stdout,
        stderr,
        timezone,
    };
    match platform {
        NativePlatform::Launchd => launchd::build(context),
        NativePlatform::Systemd => systemd::build(context),
        NativePlatform::Windows => windows::build(context),
    }
}

/// 计算指定任务在目标平台上允许写入的原生定义路径。
pub(super) fn native_paths(task_id: &str, platform: NativePlatform) -> Result<Vec<PathBuf>> {
    match platform {
        NativePlatform::Launchd => launchd::definition_paths(task_id),
        NativePlatform::Systemd => systemd::definition_paths(task_id),
        NativePlatform::Windows => windows::definition_paths(task_id),
    }
}

/// 打印每个生成文件的绝对路径和完整内容。
pub(super) fn print_artifact(artifact: &NativeArtifact) {
    for (index, file) in artifact.files.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("# {}", file.path.display());
        print!("{}", file.content);
    }
}

/// 把待注册的原生定义转换为机器输出，保留人工审核所需的完整内容。
pub(super) fn artifact_output(artifact: &NativeArtifact) -> serde_json::Value {
    json!({
        "platform": artifact.platform.label(),
        "task_id": &artifact.task_id,
        "logging": artifact.log_enabled,
        "log_directory": artifact
            .log_directory
            .as_ref()
            .map(|path| path.display().to_string()),
        "stdout": artifact
            .stdout_path
            .as_ref()
            .map(|path| path.display().to_string()),
        "stderr": artifact
            .stderr_path
            .as_ref()
            .map(|path| path.display().to_string()),
        "files": artifact.files.iter().map(|file| json!({
            "path": file.path.display().to_string(),
            "content": &file.content,
        })).collect::<Vec<_>>(),
    })
}

/// 检查磁盘文件是否逐字节等于当前声明生成的期望内容。
pub(super) fn artifact_files_match(artifact: &NativeArtifact) -> Result<bool> {
    for file in &artifact.files {
        match fs::read_to_string(&file.path) {
            Ok(content) if content == file.content => {}
            Ok(_) | Err(_) => return Ok(false),
        }
    }
    Ok(true)
}

/// 对影响注册行为的全部产物内容生成稳定摘要。
pub(super) fn artifact_digest(artifact: &NativeArtifact) -> String {
    let mut content = artifact.platform.label().to_owned();
    content.push_str(&artifact.task_id);
    for file in &artifact.files {
        content.push_str(&file.path.display().to_string());
        content.push_str(&file.content);
    }
    format!("{:016x}", fnv1a(content.as_bytes()))
}

/// 由工作区路径和计划名生成稳定且低碰撞的系统任务 ID。
pub(super) fn task_id(root: &Path, name: &str) -> String {
    format!(
        "com.batch-git.{:016x}.{}",
        fnv1a(root.display().to_string().as_bytes()),
        name.replace('_', "-")
    )
}

/// 计算稳定的 FNV-1a 64 位散列；这里只用于标识而非安全用途。
pub(super) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// 转义 plist 和 Task Scheduler XML 中的文本节点。
pub(super) fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
