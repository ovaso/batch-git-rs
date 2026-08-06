//! Schedule declaration orchestration and native scheduler integration.
#![deny(clippy::wildcard_imports)]

mod artifact;
mod commands;
mod registration;
mod state;

pub(crate) use commands::dispatch;
#[cfg(test)]
use commands::native_run_child_arguments;

use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::cli::SchedulePlatform;

/// 支持生成和注册的原生用户级调度器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativePlatform {
    Launchd,
    Systemd,
    Windows,
}

impl NativePlatform {
    /// 将 CLI 的 auto 或显式平台选择解析为确定平台。
    fn resolve(platform: SchedulePlatform) -> Result<Self> {
        match platform {
            SchedulePlatform::Launchd => Ok(Self::Launchd),
            SchedulePlatform::Systemd => Ok(Self::Systemd),
            SchedulePlatform::Windows => Ok(Self::Windows),
            SchedulePlatform::Auto => {
                #[cfg(target_os = "macos")]
                {
                    Ok(Self::Launchd)
                }
                #[cfg(target_os = "linux")]
                {
                    Ok(Self::Systemd)
                }
                #[cfg(target_os = "windows")]
                {
                    Ok(Self::Windows)
                }
                #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
                {
                    bail!("automatic schedule platform detection is unsupported on this OS")
                }
            }
        }
    }

    /// 返回写入注册摘要的稳定平台名。
    fn label(self) -> &'static str {
        match self {
            Self::Launchd => "launchd",
            Self::Systemd => "systemd",
            Self::Windows => "windows",
        }
    }

    /// 从注册摘要恢复平台枚举，并拒绝未知值。
    fn from_label(label: &str) -> Result<Self> {
        match label {
            "launchd" => Ok(Self::Launchd),
            "systemd" => Ok(Self::Systemd),
            "windows" => Ok(Self::Windows),
            _ => bail!("unsupported registered schedule platform: {label}"),
        }
    }
}

/// 一个需要写入磁盘的原生定义文件。
struct NativeFile {
    path: PathBuf,
    content: String,
}

/// 注册一次任务所需的全部原生文件和元数据。
struct NativeArtifact {
    platform: NativePlatform,
    task_id: String,
    log_enabled: bool,
    log_directory: Option<PathBuf>,
    stdout_path: Option<PathBuf>,
    stderr_path: Option<PathBuf>,
    files: Vec<NativeFile>,
}

/// batch-git 自己维护的注册事实，用于校验和安全反注册。
#[derive(Debug, Serialize, Deserialize)]
struct RegistrationState {
    workspace: String,
    name: String,
    platform: String,
    task_id: String,
    digest: String,
    files: Vec<String>,
    #[serde(default = "default_true")]
    log_enabled: bool,
    #[serde(default)]
    stdout_path: Option<String>,
    #[serde(default)]
    stderr_path: Option<String>,
    registered_at: String,
    updated_at: String,
}

/// 为旧状态文件补充日志开关默认值。
fn default_true() -> bool {
    true
}

/// 把布尔值转换为表格使用的 yes/no。
fn yes_no(value: bool) -> String {
    if value { "yes" } else { "no" }.to_owned()
}

/// 从原生调度器实时查询得到的运行状态。
struct NativeStatus {
    loaded: bool,
    state: Option<String>,
    runs: Option<u64>,
    last_exit_code: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::artifact::{
        artifact_digest, build_artifact, build_artifact_with_logging, build_artifact_with_options,
        fnv1a, launchd_cron_trigger, systemd_cron_trigger, systemd_quote, windows_argument,
        windows_repetition_interval, xml_escape,
    };
    use super::{NativePlatform, native_run_child_arguments};
    use crate::automation::AutomationOptions;
    use crate::cron::CronExpression;
    use crate::model::{ScheduleAction, ScheduleOverlap, ScheduleRecord, ScheduleScope};

    #[test]
    fn generated_identifiers_and_escaping_are_stable() {
        assert_eq!(fnv1a(b"workspace"), 0x40e26138f4336c36);
        assert_eq!(xml_escape("a&<b>"), "a&amp;&lt;b&gt;");
        assert_eq!(systemd_quote("a b"), "\"a b\"");
    }

    #[test]
    fn native_run_child_forces_non_interactive_execution() {
        let arguments =
            native_run_child_arguments("nightly-sync", 4, &AutomationOptions::default());

        assert_eq!(
            arguments,
            vec![
                "--jobs",
                "4",
                "--non-interactive",
                "schedule",
                "run",
                "nightly-sync",
            ]
        );
    }

    #[test]
    fn same_name_keeps_its_task_id_while_configuration_changes_digest() {
        let root = tempfile::tempdir().unwrap();
        let schedule = |at: &str| ScheduleRecord {
            name: "nightly-sync".to_owned(),
            enabled: true,
            action: ScheduleAction::Sync,
            at: Some(at.to_owned()),
            every: None,
            cron: None,
            timezone: "local".to_owned(),
            overlap: ScheduleOverlap::Skip,
            scope: ScheduleScope {
                all: true,
                repositories: Vec::new(),
            },
        };
        let first =
            build_artifact(root.path(), &schedule("02:30"), NativePlatform::Launchd).unwrap();
        let updated =
            build_artifact(root.path(), &schedule("03:30"), NativePlatform::Launchd).unwrap();
        assert_eq!(first.task_id, updated.task_id);
        assert_ne!(artifact_digest(&first), artifact_digest(&updated));
    }

    #[test]
    fn cron_generates_native_calendar_definitions() {
        let cron = CronExpression::parse("0 */15 9-17 * * MON-FRI").unwrap();
        let launchd = launchd_cron_trigger(&cron).unwrap();
        assert!(launchd.starts_with("<key>StartCalendarInterval</key><array>"));
        assert!(launchd.contains("<key>Minute</key><integer>15</integer>"));
        assert!(launchd.contains("<key>Hour</key><integer>9</integer>"));
        assert!(launchd.contains("<key>Weekday</key><integer>5</integer>"));
        assert_eq!(
            systemd_cron_trigger(&cron),
            "OnCalendar=Mon,Tue,Wed,Thu,Fri *-*-* 9,10,11,12,13,14,15,16,17:0,15,30,45:0"
        );

        let with_seconds = CronExpression::parse("*/10 * * * * *").unwrap();
        assert!(launchd_cron_trigger(&with_seconds).is_err());
        assert_eq!(
            systemd_cron_trigger(&with_seconds),
            "OnCalendar=*-*-* *:*:0,10,20,30,40,50"
        );
    }

    #[test]
    fn windows_generates_daily_and_interval_tasks_but_rejects_cron() {
        let root = tempfile::tempdir().unwrap();
        let schedule =
            |at: Option<&str>, every: Option<&str>, cron: Option<&str>, overlap| ScheduleRecord {
                name: "windows-sync".to_owned(),
                enabled: true,
                action: ScheduleAction::Sync,
                at: at.map(str::to_owned),
                every: every.map(str::to_owned),
                cron: cron.map(str::to_owned),
                timezone: "local".to_owned(),
                overlap,
                scope: ScheduleScope {
                    all: true,
                    repositories: Vec::new(),
                },
            };

        let daily = build_artifact(
            root.path(),
            &schedule(Some("02:30"), None, None, ScheduleOverlap::Skip),
            NativePlatform::Windows,
        )
        .unwrap();
        assert_eq!(daily.files.len(), 1);
        assert_eq!(daily.files[0].path.extension().unwrap(), "xml");
        assert!(
            daily.files[0]
                .content
                .contains("<StartBoundary>2000-01-01T02:30:00</StartBoundary>")
        );
        assert!(
            daily.files[0]
                .content
                .contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>")
        );
        assert!(
            daily.files[0]
                .content
                .contains("<Arguments>schedule run windows-sync</Arguments>")
        );

        let interval = build_artifact_with_logging(
            root.path(),
            &schedule(None, Some("15m"), None, ScheduleOverlap::Queue),
            NativePlatform::Windows,
            true,
        )
        .unwrap();
        assert!(
            interval.files[0]
                .content
                .contains("<Interval>PT15M</Interval>")
        );
        assert!(
            interval.files[0]
                .content
                .contains("<MultipleInstancesPolicy>Queue</MultipleInstancesPolicy>")
        );
        assert!(
            interval.files[0]
                .content
                .contains("<Arguments>schedule native-run windows-sync --log</Arguments>")
        );

        let timezone = build_artifact_with_options(
            root.path(),
            &schedule(Some("02:30"), None, None, ScheduleOverlap::Skip),
            NativePlatform::Windows,
            false,
            Some("Asia/Shanghai"),
        )
        .unwrap();
        assert!(timezone.files[0].content.contains(
            "<Arguments>schedule native-run windows-sync --timezone Asia/Shanghai</Arguments>"
        ));

        let cron = build_artifact(
            root.path(),
            &schedule(None, None, Some("0 0 2 * * *"), ScheduleOverlap::Skip),
            NativePlatform::Windows,
        )
        .err()
        .expect("Windows cron should be rejected");
        assert!(cron.to_string().contains("does not support cron schedules"));
        assert!(windows_repetition_interval(30).is_err());
        assert_eq!(windows_repetition_interval(90).unwrap(), "PT1M30S");
        assert_eq!(windows_repetition_interval(86_400).unwrap(), "P1D");
        assert!(windows_repetition_interval(32 * 86_400).is_err());
        assert_eq!(windows_argument("Asia/Shanghai"), "Asia/Shanghai");
        assert_eq!(windows_argument("value with space"), "\"value with space\"");
    }
}
