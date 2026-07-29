//! Portable `workspace.toml` data model and validation.

use std::collections::HashSet;
use std::path::{Component, Path};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

pub const WORKSPACE_FILE: &str = "workspace.toml";
pub const LOCK_FILE: &str = ".workspace.lock";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub version: u32,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub repositories: Vec<RepositoryRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedules: Vec<ScheduleRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryRecord {
    pub name: String,
    pub directory: String,
    pub default_branch: String,
    #[serde(default = "default_primary_remote")]
    pub primary_remote: String,
    #[serde(default)]
    pub remotes: Vec<RemoteRecord>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRecord {
    pub name: String,
    pub fetch_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleRecord {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub action: ScheduleAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    #[serde(
        default = "default_timezone",
        skip_serializing_if = "is_default_timezone"
    )]
    pub timezone: String,
    #[serde(default)]
    pub overlap: ScheduleOverlap,
    pub scope: ScheduleScope,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScheduleAction {
    #[default]
    Sync,
    Pull,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScheduleOverlap {
    #[default]
    Skip,
    Queue,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScheduleScope {
    #[serde(default)]
    pub all: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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

pub fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

impl Workspace {
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

    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("unsupported workspace version {}; expected 1", self.version);
        }
        validate_timestamp("workspace created_at", &self.created_at)?;
        validate_timestamp("workspace updated_at", &self.updated_at)?;

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
        let mut schedule_names = HashSet::new();
        for schedule in &self.schedules {
            validate_schedule_name(&schedule.name)?;
            if !schedule_names.insert(schedule.name.as_str()) {
                bail!("duplicate schedule name: {}", schedule.name);
            }
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

fn validate_schedule_interval(value: &str) -> Result<()> {
    schedule_interval_seconds(value).map(|_| ())
}

fn validate_timestamp(label: &str, value: &str) -> Result<()> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("invalid {label} timestamp: {value}"))?;
    Ok(())
}

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
