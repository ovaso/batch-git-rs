//! Runtime-only settings resolved from CLI flags and environment variables.

use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::error::{ClassifyResult, ErrorCode};
/// 返回默认并发仓库数：系统可用的逻辑 CPU 数，无法探测时保守地使用一个线程。
pub(crate) fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
}
/// 默认只扫描工作区根目录下一层。
pub(crate) const DEFAULT_SCAN_DEPTH: usize = 1;

/// 一项由 batch-git 明确支持的环境变量及其最终生效值。
#[derive(Debug)]
pub(crate) struct EnvironmentVariable {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) default: String,
    pub(crate) current: String,
}

/// 列出公开运行时环境变量，并复用真实解析器计算最终生效值。
pub(crate) fn environment_variables(resolved_jobs: usize) -> Result<Vec<EnvironmentVariable>> {
    let workspace = crate::workspace::find_root_optional()?
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "<not found>".to_owned());
    let state_directory = state_root()?.to_string_lossy().into_owned();

    Ok(vec![
        environment_variable(
            "BATCH_GIT_JOBS",
            "Maximum number of repositories operated concurrently.",
            default_jobs(),
            resolved_jobs,
        ),
        environment_variable(
            "BATCH_GIT_SCAN_DEPTH",
            "Default maximum directory depth used by scan.",
            DEFAULT_SCAN_DEPTH,
            scan_depth(None)?,
        ),
        environment_variable(
            "BATCH_GIT_WORKSPACE",
            "Absolute workspace override for commands that discover batchspace.toml.",
            "<auto-discover>",
            workspace,
        ),
        environment_variable(
            "BATCH_GIT_STATE_DIR",
            "Root directory for schedule registration state and logs.",
            default_state_directory(),
            state_directory,
        ),
        environment_variable(
            "BATCH_GIT_SCHEDULE_LOG",
            "Whether registered schedules save stdout and stderr logs.",
            false,
            schedule_log_enabled()?,
        ),
        environment_variable(
            "BATCH_GIT_TZ",
            "Timezone used when generating and running schedules.",
            "<system timezone>",
            schedule_timezone()?.unwrap_or_else(|| "<system timezone>".to_owned()),
        ),
        environment_variable(
            "BATCH_GIT_REMOTE",
            "Remote name used to disambiguate ordinary checkout and merge sources.",
            "<unset>",
            display_optional(checkout_remote(None)),
        ),
        environment_variable(
            "CURRENT_FEATURE_BRANCH",
            "Branch used by checkout --feature and merge --feature.",
            "<unset>",
            display_optional(current_feature_branch()?),
        ),
        environment_variable(
            "BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE",
            "Whether merge --default refreshes its remote source by default.",
            true,
            merge_default_refresh_source()?,
        ),
        environment_variable(
            "BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT",
            "Whether merge --feature updates the current branch by default.",
            false,
            merge_feature_update_current()?,
        ),
        environment_variable(
            "BATCH_GIT_PASSTHROUGH_VERBOSE",
            "Whether whole-workspace Git passthrough prints successful output.",
            true,
            passthrough_verbose()?,
        ),
        environment_variable(
            "NO_COLOR",
            "Disable ANSI colors when this variable is present.",
            false,
            std::env::var_os("NO_COLOR").is_some(),
        ),
    ])
}

fn environment_variable(
    name: &'static str,
    description: &'static str,
    default: impl ToString,
    current: impl ToString,
) -> EnvironmentVariable {
    EnvironmentVariable {
        name,
        description,
        default: default.to_string(),
        current: current.to_string(),
    }
}

fn display_optional(value: Option<String>) -> String {
    match value {
        Some(value) if value.is_empty() => "<empty>".to_owned(),
        Some(value) => value,
        None => "<unset>".to_owned(),
    }
}

#[cfg(target_os = "macos")]
fn default_state_directory() -> &'static str {
    "$HOME/Library/Application Support/batch-git"
}

#[cfg(target_os = "windows")]
fn default_state_directory() -> &'static str {
    "%LOCALAPPDATA%\\batch-git"
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn default_state_directory() -> &'static str {
    "$XDG_STATE_HOME/batch-git or $HOME/.local/state/batch-git"
}

/// 根据平台约定和环境变量确定 batch-git 用户状态根目录。
pub(crate) fn state_root() -> Result<PathBuf> {
    if let Some(value) = env::var_os("BATCH_GIT_STATE_DIR") {
        return Ok(PathBuf::from(value));
    }
    #[cfg(target_os = "macos")]
    {
        Ok(home_directory()?.join("Library/Application Support/batch-git"))
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(value) = env::var_os("LOCALAPPDATA").or_else(|| env::var_os("APPDATA")) {
            return Ok(PathBuf::from(value).join("batch-git"));
        }
        Ok(home_directory()?.join("AppData/Local/batch-git"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if let Some(value) = env::var_os("XDG_STATE_HOME") {
            return Ok(PathBuf::from(value).join("batch-git"));
        }
        Ok(home_directory()?.join(".local/state/batch-git"))
    }
}

/// 跨平台读取当前用户主目录，不猜测相对路径。
pub(crate) fn home_directory() -> Result<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME and USERPROFILE are not set"))
}

/// 解析并发数，优先级依次为 CLI、环境变量、默认值。
pub(crate) fn jobs(cli_value: Option<usize>) -> Result<usize> {
    let result = positive_usize("jobs", cli_value, "BATCH_GIT_JOBS", default_jobs());
    if cli_value.is_some() {
        result.classify(ErrorCode::InvalidArguments)
    } else {
        result
    }
}

/// 解析仓库扫描深度。
pub(crate) fn scan_depth(cli_value: Option<usize>) -> Result<usize> {
    positive_usize(
        "scan depth",
        cli_value,
        "BATCH_GIT_SCAN_DEPTH",
        DEFAULT_SCAN_DEPTH,
    )
}

/// 解析 checkout 时用于消除同名远端分支歧义的远端名。
pub(crate) fn checkout_remote(cli_value: Option<String>) -> Option<String> {
    cli_value.or_else(|| std::env::var("BATCH_GIT_REMOTE").ok())
}

/// 读取当前特性分支；未设置或内容为空时返回 `None`。
pub(crate) fn current_feature_branch() -> Result<Option<String>> {
    match std::env::var("CURRENT_FEATURE_BRANCH") {
        Ok(value) => {
            let value = value.trim();
            Ok((!value.is_empty()).then(|| value.to_owned()))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// 判断全工作区 Git 透传是否默认展示成功命令输出。
pub(crate) fn passthrough_verbose() -> Result<bool> {
    match std::env::var("BATCH_GIT_PASSTHROUGH_VERBOSE") {
        Ok(value) => parse_bool("BATCH_GIT_PASSTHROUGH_VERBOSE", &value),
        Err(std::env::VarError::NotPresent) => Ok(true),
        Err(error) => Err(error.into()),
    }
}

/// 判断后台定时任务是否应保存 stdout/stderr 日志。
pub(crate) fn schedule_log_enabled() -> Result<bool> {
    match std::env::var("BATCH_GIT_SCHEDULE_LOG") {
        Ok(value) => parse_bool("BATCH_GIT_SCHEDULE_LOG", &value),
        Err(std::env::VarError::NotPresent) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// 读取并校验定时任务的时区覆盖值。
pub(crate) fn schedule_timezone() -> Result<Option<String>> {
    match std::env::var("BATCH_GIT_TZ") {
        Ok(value) => {
            let value = value.trim();
            if value.chars().any(char::is_whitespace) {
                bail!("BATCH_GIT_TZ must not contain whitespace");
            }
            if !value.chars().all(|character| {
                character.is_ascii_alphanumeric()
                    || matches!(character, '/' | '_' | '+' | '-' | ':' | '.')
            }) {
                bail!("BATCH_GIT_TZ contains unsupported characters: {value}");
            }
            Ok((!value.is_empty()).then(|| value.to_owned()))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// 判断合并声明的默认分支时是否默认刷新其远端引用。
pub(crate) fn merge_default_refresh_source() -> Result<bool> {
    match std::env::var("BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE") {
        Ok(value) => parse_bool("BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE", &value),
        Err(std::env::VarError::NotPresent) => Ok(true),
        Err(error) => Err(error.into()),
    }
}

/// 判断通过 `merge --feature` 合并特性分支时是否默认先更新当前分支。
pub(crate) fn merge_feature_update_current() -> Result<bool> {
    match std::env::var("BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT") {
        Ok(value) => parse_bool("BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT", &value),
        Err(std::env::VarError::NotPresent) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// 从 CLI 或环境变量读取正整数，并为错误补充配置项名称。
fn positive_usize(
    label: &str,
    cli_value: Option<usize>,
    environment: &str,
    default: usize,
) -> Result<usize> {
    let value = match cli_value {
        Some(value) => value,
        None => match std::env::var(environment) {
            Ok(value) => value
                .parse::<usize>()
                .with_context(|| format!("invalid {environment} value: {value}"))?,
            Err(std::env::VarError::NotPresent) => default,
            Err(error) => return Err(error.into()),
        },
    };
    if value == 0 {
        bail!("{label} must be at least 1");
    }
    Ok(value)
}

/// 严格解析常见布尔写法，避免拼写错误被静默当作 false。
fn parse_bool(name: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("invalid {name} value: {value}"),
    }
}
