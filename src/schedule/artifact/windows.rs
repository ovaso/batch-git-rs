//! Windows Task Scheduler XML definition generation.

use super::*;

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
    if schedule.cron.is_some() {
        bail!("Windows Task Scheduler does not support cron schedules; use --at or --every");
    }
    let destination = windows_task_directory()?.join(format!("{task_id}.xml"));
    let trigger = if let Some(at) = &schedule.at {
        format!(
            "<CalendarTrigger><StartBoundary>2000-01-01T{at}:00</StartBoundary><Enabled>true</Enabled><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger>"
        )
    } else {
        let seconds = schedule_interval_seconds(
            schedule
                .every
                .as_deref()
                .expect("validated schedule interval"),
        )?;
        let interval = windows_repetition_interval(seconds)?;
        format!(
            "<TimeTrigger><Repetition><Interval>{interval}</Interval><StopAtDurationEnd>false</StopAtDurationEnd></Repetition><StartBoundary>2000-01-01T00:00:00</StartBoundary><Enabled>true</Enabled></TimeTrigger>"
        )
    };
    let arguments = if log_enabled || timezone.is_some() {
        let mut arguments = format!("schedule native-run {}", schedule.name);
        if log_enabled {
            arguments.push_str(" --log");
        }
        if let Some(timezone) = timezone {
            arguments.push_str(" --timezone ");
            arguments.push_str(&windows_argument(timezone));
        }
        arguments
    } else {
        format!("schedule run {}", schedule.name)
    };
    let multiple_instances = match schedule.overlap {
        ScheduleOverlap::Skip => "IgnoreNew",
        ScheduleOverlap::Queue => "Queue",
    };
    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Task version=\"1.4\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\"><RegistrationInfo><Description>batch-git schedule {}</Description></RegistrationInfo><Triggers>{trigger}</Triggers><Principals><Principal id=\"Author\"><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>{multiple_instances}</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><AllowHardTerminate>true</AllowHardTerminate><StartWhenAvailable>true</StartWhenAvailable><RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable><Enabled>true</Enabled><Hidden>false</Hidden><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><Priority>7</Priority></Settings><Actions Context=\"Author\"><Exec><Command>{}</Command><Arguments>{}</Arguments><WorkingDirectory>{}</WorkingDirectory></Exec></Actions></Task>\n",
        xml_escape(&schedule.name),
        xml_escape(&executable.display().to_string()),
        xml_escape(&arguments),
        xml_escape(&root.display().to_string()),
    );
    Ok(NativeArtifact {
        platform: NativePlatform::Windows,
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
        windows_task_directory()?.join(format!("{task_id}.xml")),
    ])
}

/// 按 Windows CommandLineToArgvW 规则引用单个命令行参数。
pub(in crate::schedule) fn windows_argument(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character == '"')
    {
        return value.to_owned();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
        } else if character == '"' {
            quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
            quoted.push('"');
            backslashes = 0;
        } else {
            quoted.push_str(&"\\".repeat(backslashes));
            backslashes = 0;
            quoted.push(character);
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

/// 把秒数转换为 Task Scheduler 接受的 ISO 8601 duration。
pub(in crate::schedule) fn windows_repetition_interval(seconds: u64) -> Result<String> {
    const MINIMUM: u64 = 60;
    const MAXIMUM: u64 = 31 * 24 * 60 * 60;
    if seconds < MINIMUM {
        bail!("Windows Task Scheduler requires --every to be at least 1m");
    }
    if seconds > MAXIMUM {
        bail!("Windows Task Scheduler requires --every to be at most 31d");
    }
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3_600;
    let minutes = seconds % 3_600 / 60;
    let seconds = seconds % 60;
    let mut value = String::from("P");
    if days > 0 {
        value.push_str(&format!("{days}D"));
    }
    if hours > 0 || minutes > 0 || seconds > 0 || days == 0 {
        value.push('T');
        if hours > 0 {
            value.push_str(&format!("{hours}H"));
        }
        if minutes > 0 {
            value.push_str(&format!("{minutes}M"));
        }
        if seconds > 0 {
            value.push_str(&format!("{seconds}S"));
        }
    }
    Ok(value)
}
