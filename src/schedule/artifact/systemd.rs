//! systemd user service and timer definition generation.

use std::path::PathBuf;

use anyhow::Result;

use super::ArtifactContext;
use crate::cron::CronExpression;
use crate::model::schedule_interval_seconds;
use crate::schedule::state::systemd_user_directory;
use crate::schedule::{NativeArtifact, NativeFile, NativePlatform};

pub(super) fn build(context: ArtifactContext<'_>) -> Result<NativeArtifact> {
    let ArtifactContext {
        root,
        schedule,
        executable,
        task_id,
        log_enabled,
        log_directory,
        stdout,
        stderr,
        timezone,
    } = context;
    let directory = systemd_user_directory()?;
    let service_path = directory.join(format!("{task_id}.service"));
    let timer_path = directory.join(format!("{task_id}.timer"));
    let timezone_environment = timezone.map_or_else(
        || "UnsetEnvironment=TZ BATCH_GIT_TZ".to_owned(),
        |timezone| {
            format!(
                "Environment={}",
                systemd_quote(&format!("BATCH_GIT_TZ={timezone}"))
            )
        },
    );
    let command_arguments = if log_enabled {
        format!(
            "schedule native-run {} --log",
            systemd_quote(&schedule.name)
        )
    } else {
        format!("schedule run {}", systemd_quote(&schedule.name))
    };
    let service = format!(
        "[Unit]\nDescription=batch-git schedule {}\n\n[Service]\nType=oneshot\nEnvironment=NO_COLOR=1\nEnvironment={}\n{timezone_environment}\nExecStart={} {command_arguments}\nStandardOutput={}\nStandardError={}\n",
        schedule.name,
        systemd_quote(&format!("BATCH_GIT_WORKSPACE={}", root.display())),
        systemd_quote(&executable.display().to_string()),
        stdout
            .as_ref()
            .map(|path| systemd_quote(&format!("append:{}", path.display())))
            .unwrap_or_else(|| "null".to_owned()),
        stderr
            .as_ref()
            .map(|path| systemd_quote(&format!("append:{}", path.display())))
            .unwrap_or_else(|| "null".to_owned())
    );
    let timezone_suffix = timezone.map_or_else(String::new, |timezone| format!(" {timezone}"));
    let timer_trigger = if let Some(at) = &schedule.at {
        format!("OnCalendar=*-*-* {at}:00{timezone_suffix}")
    } else if let Some(every) = &schedule.every {
        format!(
            "OnUnitActiveSec={}s\nOnBootSec={}s",
            schedule_interval_seconds(every)?,
            schedule_interval_seconds(every)?
        )
    } else {
        format!(
            "{}{timezone_suffix}",
            systemd_cron_trigger(&CronExpression::parse(
                schedule.cron.as_deref().expect("validated cron"),
            )?)
        )
    };
    let timer = format!(
        "[Unit]\nDescription=batch-git schedule {}\n\n[Timer]\n{}\nPersistent=true\nUnit={}.service\n\n[Install]\nWantedBy=timers.target\n",
        schedule.name, timer_trigger, task_id
    );
    Ok(NativeArtifact {
        platform: NativePlatform::Systemd,
        task_id,
        log_enabled,
        log_directory,
        stdout_path: stdout,
        stderr_path: stderr,
        files: vec![
            NativeFile {
                path: service_path,
                content: service,
            },
            NativeFile {
                path: timer_path,
                content: timer,
            },
        ],
    })
}

pub(super) fn definition_paths(task_id: &str) -> Result<Vec<PathBuf>> {
    let directory = systemd_user_directory()?;
    Ok(vec![
        directory.join(format!("{task_id}.service")),
        directory.join(format!("{task_id}.timer")),
    ])
}

/// 把展开后的 cron 集合压缩为 systemd OnCalendar 文本。
pub(in crate::schedule) fn systemd_cron_trigger(cron: &CronExpression) -> String {
    let weekday = if cron.days_of_week_unrestricted() {
        String::new()
    } else {
        format!(
            "{} ",
            cron.days_of_week()
                .iter()
                .map(|value| match value {
                    0 => "Sun",
                    1 => "Mon",
                    2 => "Tue",
                    3 => "Wed",
                    4 => "Thu",
                    5 => "Fri",
                    6 => "Sat",
                    _ => unreachable!("validated weekday"),
                })
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let month = cron_component(cron.months(), cron.months_unrestricted());
    let day = cron_component(cron.days_of_month(), cron.days_of_month_unrestricted());
    format!(
        "OnCalendar={weekday}*-{month}-{day} {}:{}:{}",
        cron_component(cron.hours(), cron.hours_unrestricted()),
        cron_component(cron.minutes(), cron.minutes_unrestricted()),
        cron_component(cron.seconds(), cron.seconds_unrestricted())
    )
}

/// 生成一个 systemd 日历字段，未限制字段输出星号。
fn cron_component(values: &[u8], unrestricted: bool) -> String {
    if unrestricted {
        "*".to_owned()
    } else {
        join_numbers(values)
    }
}

/// 将有序数字集合连接为逗号分隔文本。
fn join_numbers(values: &[u8]) -> String {
    values
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// 引用 systemd Environment/ExecStart 中的单个值。
pub(in crate::schedule) fn systemd_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
