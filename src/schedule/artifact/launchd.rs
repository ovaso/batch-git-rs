//! launchd plist definition generation.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::{ArtifactContext, xml_escape};
use crate::cron::CronExpression;
use crate::model::schedule_interval_seconds;
use crate::schedule::{NativeArtifact, NativeFile, NativePlatform};
use crate::settings::home_directory;

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
    let destination = home_directory()?
        .join("Library/LaunchAgents")
        .join(format!("{task_id}.plist"));
    let trigger = if let Some(at) = &schedule.at {
        let (hour, minute) = at.split_once(':').expect("validated schedule time");
        format!(
            "<key>StartCalendarInterval</key><dict><key>Hour</key><integer>{}</integer><key>Minute</key><integer>{}</integer></dict>",
            hour.parse::<u8>()?,
            minute.parse::<u8>()?
        )
    } else if let Some(every) = &schedule.every {
        format!(
            "<key>StartInterval</key><integer>{}</integer>",
            schedule_interval_seconds(every)?
        )
    } else {
        launchd_cron_trigger(&CronExpression::parse(
            schedule.cron.as_deref().expect("validated cron"),
        )?)?
    };
    let timezone_environment = timezone.map_or_else(String::new, |timezone| {
        format!(
            "<key>BATCH_GIT_TZ</key><string>{}</string>",
            xml_escape(timezone)
        )
    });
    let command_arguments = if log_enabled {
        format!(
            "<string>schedule</string><string>native-run</string><string>{}</string><string>--log</string>",
            xml_escape(&schedule.name)
        )
    } else {
        format!(
            "<string>schedule</string><string>run</string><string>{}</string>",
            xml_escape(&schedule.name)
        )
    };
    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>ProgramArguments</key><array><string>{}</string>{command_arguments}</array><key>EnvironmentVariables</key><dict><key>BATCH_GIT_WORKSPACE</key><string>{}</string><key>NO_COLOR</key><string>1</string>{timezone_environment}</dict>{}<key>StandardOutPath</key><string>{}</string><key>StandardErrorPath</key><string>{}</string></dict></plist>\n",
        xml_escape(&task_id),
        xml_escape(&executable.display().to_string()),
        xml_escape(&root.display().to_string()),
        trigger,
        xml_escape(
            &stdout
                .as_deref()
                .unwrap_or(Path::new("/dev/null"))
                .display()
                .to_string()
        ),
        xml_escape(
            &stderr
                .as_deref()
                .unwrap_or(Path::new("/dev/null"))
                .display()
                .to_string()
        )
    );
    Ok(NativeArtifact {
        platform: NativePlatform::Launchd,
        task_id,
        log_enabled,
        log_directory,
        stdout_path: stdout,
        stderr_path: stderr,
        files: vec![NativeFile {
            path: destination,
            content,
        }],
    })
}

pub(super) fn definition_paths(task_id: &str) -> Result<Vec<PathBuf>> {
    Ok(vec![
        home_directory()?
            .join("Library/LaunchAgents")
            .join(format!("{task_id}.plist")),
    ])
}

/// 将 cron 展开为 launchd 的 StartCalendarInterval 字典数组。
pub(in crate::schedule) fn launchd_cron_trigger(cron: &CronExpression) -> Result<String> {
    if cron.seconds() != [0] {
        bail!(
            "launchd cron schedules require the second field to be exactly 0; use systemd or an every interval for sub-minute schedules"
        );
    }
    let hours = optional_cron_values(cron.hours(), cron.hours_unrestricted());
    let days = optional_cron_values(cron.days_of_month(), cron.days_of_month_unrestricted());
    let months = optional_cron_values(cron.months(), cron.months_unrestricted());
    let weekdays = optional_cron_values(cron.days_of_week(), cron.days_of_week_unrestricted());
    let count = cron.minutes().len()
        * hours.as_ref().map_or(1, Vec::len)
        * days.as_ref().map_or(1, Vec::len)
        * months.as_ref().map_or(1, Vec::len)
        * weekdays.as_ref().map_or(1, Vec::len);
    if count > 4096 {
        bail!(
            "cron expression expands to {count} launchd calendar entries; maximum supported is 4096"
        );
    }

    let mut dictionaries = Vec::with_capacity(count);
    for minute in cron.minutes() {
        for hour in optional_iter(&hours) {
            for day in optional_iter(&days) {
                for month in optional_iter(&months) {
                    for weekday in optional_iter(&weekdays) {
                        let mut dictionary = String::from("<dict>");
                        push_launchd_integer(&mut dictionary, "Minute", Some(*minute));
                        push_launchd_integer(&mut dictionary, "Hour", hour);
                        push_launchd_integer(&mut dictionary, "Day", day);
                        push_launchd_integer(&mut dictionary, "Month", month);
                        push_launchd_integer(&mut dictionary, "Weekday", weekday);
                        dictionary.push_str("</dict>");
                        dictionaries.push(dictionary);
                    }
                }
            }
        }
    }
    let value = if dictionaries.len() == 1 {
        dictionaries.pop().expect("one launchd calendar entry")
    } else {
        format!("<array>{}</array>", dictionaries.concat())
    };
    Ok(format!("<key>StartCalendarInterval</key>{value}"))
}

/// 未限制字段用 None 表示，避免生成无意义的全值笛卡尔积。
fn optional_cron_values(values: &[u8], unrestricted: bool) -> Option<Vec<u8>> {
    (!unrestricted).then(|| values.to_vec())
}

/// 把可选字段转换为至少含一个元素的迭代集合。
fn optional_iter(values: &Option<Vec<u8>>) -> Vec<Option<u8>> {
    match values {
        Some(values) => values.iter().copied().map(Some).collect(),
        None => vec![None],
    }
}

/// 仅在字段受限制时写入 launchd 整数字段。
fn push_launchd_integer(output: &mut String, key: &str, value: Option<u8>) {
    if let Some(value) = value {
        output.push_str(&format!("<key>{key}</key><integer>{value}</integer>"));
    }
}
