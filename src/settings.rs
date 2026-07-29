//! Runtime-only settings resolved from CLI flags and environment variables.

use anyhow::{Context, Result, bail};

pub(crate) const DEFAULT_JOBS: usize = 4;
pub(crate) const DEFAULT_SCAN_DEPTH: usize = 1;

pub(crate) fn jobs(cli_value: Option<usize>) -> Result<usize> {
    positive_usize("jobs", cli_value, "BATCH_GIT_JOBS", DEFAULT_JOBS)
}

pub(crate) fn scan_depth(cli_value: Option<usize>) -> Result<usize> {
    positive_usize(
        "scan depth",
        cli_value,
        "BATCH_GIT_SCAN_DEPTH",
        DEFAULT_SCAN_DEPTH,
    )
}

pub(crate) fn checkout_remote(cli_value: Option<String>) -> Option<String> {
    cli_value.or_else(|| std::env::var("BATCH_GIT_REMOTE").ok())
}

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

pub(crate) fn passthrough_verbose() -> Result<bool> {
    match std::env::var("BATCH_GIT_PASSTHROUGH_VERBOSE") {
        Ok(value) => parse_bool("BATCH_GIT_PASSTHROUGH_VERBOSE", &value),
        Err(std::env::VarError::NotPresent) => Ok(true),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn schedule_log_enabled() -> Result<bool> {
    match std::env::var("BATCH_GIT_SCHEDULE_LOG") {
        Ok(value) => parse_bool("BATCH_GIT_SCHEDULE_LOG", &value),
        Err(std::env::VarError::NotPresent) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

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

pub(crate) fn merge_update_current(cli_value: Option<bool>) -> Result<bool> {
    if let Some(value) = cli_value {
        return Ok(value);
    }
    match std::env::var("BATCH_GIT_MERGE_UPDATE_CURRENT") {
        Ok(value) => parse_bool("BATCH_GIT_MERGE_UPDATE_CURRENT", &value),
        Err(std::env::VarError::NotPresent) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

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

fn parse_bool(name: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("invalid {name} value: {value}"),
    }
}
