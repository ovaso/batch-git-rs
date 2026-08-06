//! Invocation-wide automation settings and validation.

use std::time::Duration;

use anyhow::{Result, bail};
use clap::ValueEnum;

use crate::error::{ClassifyResult, ErrorCode};

/// Output rendering selected by the global `--output` option.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum OutputFormat {
    #[default]
    Text,
    Json,
    Jsonl,
}

impl OutputFormat {
    pub(crate) fn is_machine(self) -> bool {
        !matches!(self, Self::Text)
    }
}

/// Invocation-wide behavior shared by every child operation.
#[derive(Debug, Clone)]
pub(crate) struct AutomationOptions {
    pub(crate) output: OutputFormat,
    pub(crate) request_id: Option<String>,
    pub(crate) non_interactive: bool,
    pub(crate) timeout: Option<Duration>,
    pub(crate) plan: bool,
    pub(crate) apply: bool,
    pub(crate) expected_workspace_revision: Option<String>,
}

impl Default for AutomationOptions {
    fn default() -> Self {
        Self {
            output: OutputFormat::Text,
            request_id: None,
            non_interactive: false,
            timeout: None,
            plan: false,
            apply: false,
            expected_workspace_revision: None,
        }
    }
}

impl AutomationOptions {
    pub(crate) fn is_machine(&self) -> bool {
        self.output.is_machine()
    }

    pub(crate) fn validate(&self) -> Result<()> {
        (|| {
            if let Some(request_id) = &self.request_id {
                validate_request_id(request_id)?;
            }
            if self.plan && self.apply {
                bail!("--plan conflicts with --apply");
            }
            if self.apply && self.expected_workspace_revision.is_none() {
                bail!("--apply requires --expect-workspace-revision");
            }
            if !self.apply && self.expected_workspace_revision.is_some() {
                bail!("--expect-workspace-revision requires --apply");
            }
            Ok(())
        })()
        .classify(ErrorCode::InvalidArguments)
    }
}

/// Parse the compact duration syntax accepted by `--timeout`.
pub(crate) fn parse_timeout(raw: &str) -> Result<Duration> {
    parse_timeout_inner(raw).classify(ErrorCode::InvalidArguments)
}

fn parse_timeout_inner(raw: &str) -> Result<Duration> {
    let raw = raw.trim();
    let Some((number, unit)) = raw
        .char_indices()
        .find(|(_, character)| !character.is_ascii_digit())
        .map(|(index, _)| raw.split_at(index))
    else {
        bail!("timeout must use a unit such as 30s, 5m, or 1h");
    };
    if number.is_empty() || unit.len() != 1 {
        bail!("invalid timeout: {raw}");
    }
    let value = number
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("invalid timeout: {raw}"))?;
    if value == 0 {
        bail!("timeout must be greater than zero");
    }
    let multiplier = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        _ => bail!("timeout unit must be s, m, or h: {raw}"),
    };
    let seconds = value
        .checked_mul(multiplier)
        .ok_or_else(|| anyhow::anyhow!("timeout is too large: {raw}"))?;
    Ok(Duration::from_secs(seconds))
}

pub(super) fn validate_request_id(request_id: &str) -> Result<()> {
    if request_id.is_empty()
        || request_id.chars().count() > 128
        || request_id.chars().any(char::is_control)
    {
        return Err(crate::error::classified(
            ErrorCode::InvalidArguments,
            "request id must contain 1 to 128 non-control characters",
        ));
    }
    Ok(())
}
