//! Portable `batchspace.toml` data model and validation.

use std::collections::HashSet;
use std::path::{Component, Path};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

/// 工作区清单的固定文件名。
pub const WORKSPACE_FILE: &str = "batchspace.toml";
/// 串行化工作区写操作的锁文件名。
pub const LOCK_FILE: &str = ".batchspace.lock";
/// 仅用于与重命名前版本协调的旧锁文件名。
pub const LEGACY_LOCK_FILE: &str = ".workspace.lock";

/// 可序列化、可复制的完整工作区声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    /// 清单格式版本，用于未来进行兼容性迁移。
    pub version: u32,
    /// 工作区首次创建时间，使用 RFC 3339 UTC 字符串。
    pub created_at: String,
    /// 清单最近一次成功写入时间。
    pub updated_at: String,
    #[serde(default)]
    /// 按相对目录登记的仓库集合。
    pub repositories: Vec<RepositoryRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// 可选的后台同步计划。
    pub schedules: Vec<ScheduleRecord>,
}

/// 恢复一个仓库所需的稳定声明，不保存实时 Git 状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryRecord {
    /// 面向用户的唯一仓库名。
    pub name: String,
    /// 相对于工作区根目录的路径。
    pub directory: String,
    /// `cd` 和默认 checkout 使用的分支。
    pub default_branch: String,
    #[serde(default = "default_primary_remote")]
    /// clone、首次 push 等操作默认使用的远端。
    pub primary_remote: String,
    #[serde(default)]
    /// 仓库声明的全部远端及其地址。
    pub remotes: Vec<RemoteRecord>,
    /// 仓库首次被登记的时间。
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// 最近一次由 batch-git 成功同步远端配置的时间。
    pub synced_at: Option<String>,
}

/// 一个 Git remote 的拉取地址和可选独立推送地址。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRecord {
    /// Git 配置中的 remote 名称。
    pub name: String,
    /// fetch/clone 使用的地址。
    pub fetch_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// 仅在与 fetch 地址不同时保存的 push 地址。
    pub push_url: Option<String>,
}

/// 一个跨平台定时同步声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleRecord {
    /// 工作区内唯一、可安全用于文件名的计划名称。
    pub name: String,
    #[serde(default = "default_true")]
    /// 禁用后不能生成、注册或运行。
    pub enabled: bool,
    #[serde(default)]
    /// 计划触发时执行安全 sync 还是 fast-forward pull。
    pub action: ScheduleAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// 每日 `HH:MM` 触发器，与 `every`、`cron` 互斥。
    pub at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// 固定间隔触发器，如 `30m`。
    pub every: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// 六段式 cron 触发器。
    pub cron: Option<String>,
    #[serde(
        default = "default_timezone",
        skip_serializing_if = "is_default_timezone"
    )]
    /// 清单中的兼容字段；当前只允许 `local`。
    pub timezone: String,
    #[serde(default)]
    /// 与其他工作区操作重叠时的处理策略。
    pub overlap: ScheduleOverlap,
    /// 计划覆盖全部仓库或指定仓库。
    pub scope: ScheduleScope,
}

/// 定时任务实际执行的工作区动作。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScheduleAction {
    /// 恢复缺失仓库并 fetch，不改变已有工作树。
    #[default]
    Sync,
    /// 对 tracking 分支执行安全的 fast-forward-only pull。
    Pull,
}

/// 当工作区锁已被占用时的策略。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScheduleOverlap {
    /// 立即跳过本轮执行。
    #[default]
    Skip,
    /// 阻塞等待现有工作区操作完成。
    Queue,
}

/// 定时任务的仓库选择范围。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScheduleScope {
    #[serde(default)]
    /// 为 true 时选择清单中的全部仓库。
    pub all: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// `all` 为 false 时保存规范化后的仓库名。
    pub repositories: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_timezone() -> String {
    "local".to_owned()
}

fn is_default_timezone(value: &str) -> bool {
    value == "local"
}

fn default_primary_remote() -> String {
    "origin".to_owned()
}

/// 返回秒精度、UTC 的 RFC 3339 时间戳。
pub fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

impl Workspace {
    /// 创建空的版本 1 工作区，并让创建与更新时间一致。
    pub fn new() -> Self {
        let timestamp = now();
        Self {
            version: 1,
            created_at: timestamp.clone(),
            updated_at: timestamp,
            repositories: Vec::new(),
            schedules: Vec::new(),
        }
    }

    /// 验证清单的格式、唯一性、引用完整性和跨字段约束。
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("unsupported workspace version {}; expected 1", self.version);
        }
        validate_timestamp("workspace created_at", &self.created_at)?;
        validate_timestamp("workspace updated_at", &self.updated_at)?;

        // 名称和目录都必须唯一，否则用户选择器会产生歧义。
        let mut names = HashSet::new();
        let mut directories = HashSet::new();
        for repository in &self.repositories {
            if repository.name.trim().is_empty() {
                bail!("repository name cannot be empty");
            }
            if !names.insert(repository.name.clone()) {
                bail!("duplicate repository name: {}", repository.name);
            }
            validate_directory(&repository.directory)
                .with_context(|| format!("invalid directory for {}", repository.name))?;
            if !directories.insert(repository.directory.clone()) {
                bail!("duplicate repository directory: {}", repository.directory);
            }
            if repository.default_branch.trim().is_empty() {
                bail!("repository {} has no default_branch", repository.name);
            }
            validate_timestamp(
                &format!("repository {} created_at", repository.name),
                &repository.created_at,
            )?;
            if let Some(timestamp) = &repository.synced_at {
                validate_timestamp(
                    &format!("repository {} synced_at", repository.name),
                    timestamp,
                )?;
            }
            if repository.remotes.is_empty() {
                bail!("repository {} has no remotes", repository.name);
            }
            let mut remote_names = HashSet::new();
            for remote in &repository.remotes {
                if remote.name.trim().is_empty() || remote.fetch_url.trim().is_empty() {
                    bail!("repository {} has an invalid remote", repository.name);
                }
                if !remote_names.insert(remote.name.as_str()) {
                    bail!(
                        "repository {} has duplicate remote {}",
                        repository.name,
                        remote.name
                    );
                }
            }
            if !remote_names.contains(repository.primary_remote.as_str()) {
                bail!(
                    "repository {} primary remote {} is not declared",
                    repository.name,
                    repository.primary_remote
                );
            }
        }
        // schedule 名称最终会进入状态文件名和系统任务 ID，因此必须唯一且可移植。
        let mut schedule_names = HashSet::new();
        for schedule in &self.schedules {
            validate_schedule_name(&schedule.name)?;
            if !schedule_names.insert(schedule.name.as_str()) {
                bail!("duplicate schedule name: {}", schedule.name);
            }
            // 三种触发器严格互斥，避免不同平台选择不同解释。
            let trigger_count = usize::from(schedule.at.is_some())
                + usize::from(schedule.every.is_some())
                + usize::from(schedule.cron.is_some());
            if trigger_count != 1 {
                bail!(
                    "schedule {} must declare exactly one of at, every, or cron",
                    schedule.name
                );
            }
            if let Some(at) = &schedule.at {
                validate_schedule_time(at)
                    .with_context(|| format!("invalid schedule {} at", schedule.name))?;
            }
            if let Some(every) = &schedule.every {
                validate_schedule_interval(every)
                    .with_context(|| format!("invalid schedule {} every", schedule.name))?;
            }
            if let Some(cron) = &schedule.cron {
                crate::cron::CronExpression::parse(cron)
                    .with_context(|| format!("invalid schedule {} cron", schedule.name))?;
            }
            if schedule.timezone != "local" {
                bail!(
                    "schedule {} timezone is not supported yet: {}",
                    schedule.name,
                    schedule.timezone
                );
            }
            if schedule.scope.all != schedule.scope.repositories.is_empty() {
                bail!(
                    "schedule {} scope must select either all repositories or named repositories",
                    schedule.name
                );
            }
            for selector in &schedule.scope.repositories {
                let count = self
                    .repositories
                    .iter()
                    .filter(|repository| {
                        repository.name == *selector || repository.directory == *selector
                    })
                    .count();
                match count {
                    1 => {}
                    0 => bail!(
                        "schedule {} references unknown repository: {}",
                        schedule.name,
                        selector
                    ),
                    _ => bail!(
                        "schedule {} has ambiguous repository selector: {}",
                        schedule.name,
                        selector
                    ),
                }
            }
        }
        Ok(())
    }
}

/// 校验严格的 24 小时制 `HH:MM` 文本。
fn validate_schedule_time(value: &str) -> Result<()> {
    let Some((hour, minute)) = value.split_once(':') else {
        bail!("expected HH:MM");
    };
    if hour.len() != 2 || minute.len() != 2 {
        bail!("expected HH:MM");
    }
    let hour = hour.parse::<u8>().context("hour is not a number")?;
    let minute = minute.parse::<u8>().context("minute is not a number")?;
    if hour > 23 || minute > 59 {
        bail!("time is outside 00:00 through 23:59");
    }
    Ok(())
}

/// 校验可安全用于跨平台任务标识符和文件名的 schedule 名称。
pub(crate) fn validate_schedule_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("schedule name must contain only ASCII letters, digits, '-' or '_': {name}");
    }
    Ok(())
}

/// 将 `30m`、`6h` 等固定间隔安全换算为秒。
pub(crate) fn schedule_interval_seconds(value: &str) -> Result<u64> {
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .ok_or_else(|| anyhow::anyhow!("interval requires a unit: s, m, h, or d"))?;
    let (amount, unit) = value.split_at(split);
    if amount.is_empty() || unit.len() != 1 {
        bail!("expected a positive integer followed by s, m, h, or d");
    }
    let amount = amount.parse::<u64>().context("invalid interval amount")?;
    if amount == 0 {
        bail!("interval must be positive");
    }
    let multiplier = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        _ => bail!("unsupported interval unit: {unit}"),
    };
    amount
        .checked_mul(multiplier)
        .ok_or_else(|| anyhow::anyhow!("interval is too large"))
}

/// 仅验证固定间隔，不保留换算结果。
fn validate_schedule_interval(value: &str) -> Result<()> {
    schedule_interval_seconds(value).map(|_| ())
}

/// 验证清单时间字段为 RFC 3339，并在错误中保留字段标签。
fn validate_timestamp(label: &str, value: &str) -> Result<()> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("invalid {label} timestamp: {value}"))?;
    Ok(())
}

/// 验证仓库目录为不能逃逸工作区的规范相对路径。
pub fn validate_directory(directory: &str) -> Result<()> {
    let path = Path::new(directory);
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("directory must be a non-empty relative path");
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => bail!("directory cannot contain '.', '..', root, or prefix components"),
        }
    }
    Ok(())
}
