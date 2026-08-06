//! Registration state persistence, validation, and live status queries.

use super::*;

/// 查询任务是否加载及最近退出状态；任务不存在不是致命错误。
pub(super) fn native_status(state: &RegistrationState) -> Result<NativeStatus> {
    match state.platform.as_str() {
        "launchd" => {
            let target = format!("{}/{}", launchd_domain()?, state.task_id);
            let output = Command::new("launchctl")
                .args(["print", &target])
                .output()
                .context("failed to execute launchctl")?;
            let text = String::from_utf8_lossy(&output.stdout);
            Ok(NativeStatus {
                loaded: output.status.success(),
                state: parse_native_value(&text, "state"),
                runs: parse_native_value(&text, "runs").and_then(|value| value.parse().ok()),
                last_exit_code: parse_native_value(&text, "last exit code")
                    .and_then(|value| value.parse().ok()),
            })
        }
        "systemd" => {
            let timer = format!("{}.timer", state.task_id);
            let timer_output = Command::new("systemctl")
                .args([
                    "--user",
                    "show",
                    &timer,
                    "--property=LoadState",
                    "--property=ActiveState",
                    "--property=SubState",
                ])
                .output()
                .context("failed to execute systemctl")?;
            let timer_text = String::from_utf8_lossy(&timer_output.stdout);
            let service = format!("{}.service", state.task_id);
            let service_output = Command::new("systemctl")
                .args([
                    "--user",
                    "show",
                    &service,
                    "--property=Result",
                    "--property=ExecMainStatus",
                    "--property=InvocationID",
                ])
                .output()
                .context("failed to execute systemctl")?;
            let service_text = String::from_utf8_lossy(&service_output.stdout);
            let active = parse_systemd_value(&timer_text, "ActiveState");
            let sub = parse_systemd_value(&timer_text, "SubState");
            Ok(NativeStatus {
                loaded: timer_output.status.success()
                    && parse_systemd_value(&timer_text, "LoadState").as_deref() == Some("loaded"),
                state: match (active, sub) {
                    (Some(active), Some(sub)) => Some(format!("{active}/{sub}")),
                    (active, sub) => active.or(sub),
                },
                runs: None,
                last_exit_code: if service_output.status.success()
                    && parse_systemd_value(&service_text, "InvocationID")
                        .is_some_and(|value| !value.is_empty())
                {
                    parse_systemd_value(&service_text, "ExecMainStatus")
                        .and_then(|value| value.parse().ok())
                } else {
                    None
                },
            })
        }
        "windows" => {
            let output = Command::new("schtasks.exe")
                .args(["/Query", "/TN", &state.task_id])
                .output()
                .context("failed to execute schtasks.exe")?;
            Ok(NativeStatus {
                loaded: output.status.success(),
                state: None,
                runs: None,
                last_exit_code: None,
            })
        }
        platform => bail!("unsupported registered schedule platform: {platform}"),
    }
}

/// 从 `key = value` 风格原生命令输出中提取字段。
fn parse_native_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (name, value) = line.trim().split_once('=')?;
        (name.trim() == key).then(|| value.trim().to_owned())
    })
}

/// 从 systemctl show 的 `Key=Value` 输出中提取字段。
fn parse_systemd_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name == key).then(|| value.to_owned())
    })
}

/// 读取并验证本地注册摘要；文件不存在表示尚未注册。
pub(super) fn load_state(root: &Path, name: &str) -> Result<Option<RegistrationState>> {
    let path = registration_state_path(root, name)?;
    if !path.is_file() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut state: RegistrationState = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if state.log_enabled && (state.stdout_path.is_none() || state.stderr_path.is_none()) {
        let directory = schedule_log_directory(root, name)?;
        state.stdout_path = Some(directory.join("stdout.log").display().to_string());
        state.stderr_path = Some(directory.join("stderr.log").display().to_string());
    }
    validate_state(root, name, &state)?;
    Ok(Some(state))
}

/// 原子持久化注册摘要。
pub(super) fn write_state(root: &Path, state: &RegistrationState) -> Result<()> {
    let path = registration_state_path(root, &state.name)?;
    atomic_write(&path, serde_json::to_string_pretty(state)?.as_bytes())
}

/// 计算当前工作区和计划对应的状态文件路径。
pub(super) fn registration_state_path(root: &Path, name: &str) -> Result<PathBuf> {
    crate::model::validate_schedule_name(name)?;
    Ok(state_root()?
        .join("registrations")
        .join(format!(
            "{:016x}",
            fnv1a(root.display().to_string().as_bytes())
        ))
        .join(format!("{name}.json")))
}

/// 计算隔离到工作区和计划名称的日志目录。
pub(super) fn schedule_log_directory(root: &Path, name: &str) -> Result<PathBuf> {
    crate::model::validate_schedule_name(name)?;
    Ok(state_root()?
        .join("logs")
        .join(format!(
            "{:016x}",
            fnv1a(root.display().to_string().as_bytes())
        ))
        .join(name))
}

/// 返回 systemd 用户单元目录。
pub(super) fn systemd_user_directory() -> Result<PathBuf> {
    if let Some(value) = env::var_os("XDG_CONFIG_HOME") {
        Ok(PathBuf::from(value).join("systemd/user"))
    } else {
        Ok(home_directory()?.join(".config/systemd/user"))
    }
}

/// 返回保存 Windows Task Scheduler XML 副本的目录。
pub(super) fn windows_task_directory() -> Result<PathBuf> {
    Ok(state_root()?.join("tasks/windows"))
}

/// 防止损坏状态文件引导程序操作其他工作区的任务。
fn validate_state(root: &Path, name: &str, state: &RegistrationState) -> Result<()> {
    if state.workspace != root.display().to_string() || state.name != name {
        bail!("schedule registration state does not match this workspace and schedule");
    }
    let platform = NativePlatform::from_label(&state.platform)?;
    if state.task_id != task_id(root, name) {
        bail!("schedule registration contains an invalid task id");
    }
    let expected = native_paths(&state.task_id, platform)?;
    let actual = state.files.iter().map(PathBuf::from).collect::<Vec<_>>();
    if actual != expected {
        bail!("schedule registration contains unexpected native task paths");
    }
    Ok(())
}
