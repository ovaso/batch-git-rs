//! Versioned machine-output protocol and agent execution options.

use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail};
use clap::ValueEnum;
use serde::Serialize;
use serde_json::Value;

/// Stable protocol version emitted by `--output json` and `--output jsonl`.
pub(crate) const API_VERSION: &str = "v1";

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

/// Invocation-wide behavior that must remain consistent across all child operations.
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
    }
}

/// Parse the compact duration syntax accepted by `--timeout`.
pub(crate) fn parse_timeout(raw: &str) -> Result<Duration> {
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

fn validate_request_id(request_id: &str) -> Result<()> {
    if request_id.is_empty()
        || request_id.chars().count() > 128
        || request_id.chars().any(char::is_control)
    {
        bail!("request id must contain 1 to 128 non-control characters");
    }
    Ok(())
}

#[derive(Clone, Serialize)]
struct WorkspaceContext {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
}

#[derive(Serialize)]
struct Envelope {
    api_version: &'static str,
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<WorkspaceContext>,
    exit_code: i32,
    ok: bool,
    data: Value,
    error: Option<MachineError>,
}

#[derive(Serialize)]
struct MachineError {
    code: &'static str,
    message: String,
    retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<&'static str>,
}

#[derive(Serialize)]
struct JsonlEvent {
    api_version: &'static str,
    event: &'static str,
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<WorkspaceContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ok: Option<bool>,
    data: Value,
}

/// Emit the final response for a completed command. Call only in machine modes.
pub(crate) fn emit_data<T: Serialize>(
    options: &AutomationOptions,
    command: &str,
    root: Option<&Path>,
    exit_code: i32,
    data: &T,
) -> Result<()> {
    let data = serde_json::to_value(data)?;
    emit_data_with_workspace_context(options, command, workspace_context(root), exit_code, data)
}

/// Emit a response using a manifest revision captured together with the planned workspace.
///
/// Planning must never recompute the revision after it has resolved its selection: a concurrent
/// manifest edit would otherwise attach a newer digest to an older selection.
pub(crate) fn emit_data_with_workspace_revision<T: Serialize>(
    options: &AutomationOptions,
    command: &str,
    root: Option<&Path>,
    workspace_revision: Option<&str>,
    exit_code: i32,
    data: &T,
) -> Result<()> {
    let data = serde_json::to_value(data)?;
    emit_data_with_workspace_context(
        options,
        command,
        workspace_context_with_revision(root, workspace_revision),
        exit_code,
        data,
    )
}

fn emit_data_with_workspace_context(
    options: &AutomationOptions,
    command: &str,
    workspace: Option<WorkspaceContext>,
    exit_code: i32,
    data: Value,
) -> Result<()> {
    match options.output {
        OutputFormat::Text => Ok(()),
        OutputFormat::Json => {
            let envelope = Envelope {
                api_version: API_VERSION,
                command: command.to_owned(),
                request_id: options.request_id.clone(),
                workspace,
                exit_code,
                ok: exit_code == 0,
                data,
                error: None,
            };
            println!("{}", serde_json::to_string(&envelope)?);
            Ok(())
        }
        OutputFormat::Jsonl => {
            emit_event_with_workspace(
                options,
                "started",
                command,
                workspace.clone(),
                None,
                None,
                Value::Null,
            )?;
            emit_event_with_workspace(
                options,
                "finished",
                command,
                workspace,
                Some(exit_code),
                Some(exit_code == 0),
                data,
            )
        }
    }
}

/// Emit one JSONL event. This is intentionally a no-op for non-JSONL modes.
pub(crate) fn emit_event(
    options: &AutomationOptions,
    event: &'static str,
    command: &str,
    root: Option<&Path>,
    data: Value,
) -> Result<()> {
    if options.output != OutputFormat::Jsonl {
        return Ok(());
    }
    emit_event_with_workspace(
        options,
        event,
        command,
        workspace_context(root),
        None,
        None,
        data,
    )
}

/// Emit a final JSONL event with the aggregate process result.
pub(crate) fn emit_finished(
    options: &AutomationOptions,
    command: &str,
    root: Option<&Path>,
    exit_code: i32,
    data: Value,
) -> Result<()> {
    emit_event_with_workspace(
        options,
        "finished",
        command,
        workspace_context(root),
        Some(exit_code),
        Some(exit_code == 0),
        data,
    )
}

fn emit_event_with_workspace(
    options: &AutomationOptions,
    event: &'static str,
    command: &str,
    workspace: Option<WorkspaceContext>,
    exit_code: Option<i32>,
    ok: Option<bool>,
    data: Value,
) -> Result<()> {
    if options.output != OutputFormat::Jsonl {
        return Ok(());
    }
    let event = JsonlEvent {
        api_version: API_VERSION,
        event,
        command: command.to_owned(),
        request_id: options.request_id.clone(),
        workspace,
        exit_code,
        ok,
        data,
    };
    println!("{}", serde_json::to_string(&event)?);
    Ok(())
}

/// Emit a top-level failure without contaminating machine-mode stdout with text diagnostics.
pub(crate) fn emit_error(
    options: &AutomationOptions,
    command: Option<&str>,
    error: &anyhow::Error,
) -> Result<()> {
    let message = sanitize_message(&format!("{error:#}"));
    let descriptor = error_descriptor(&message);
    let machine_error = MachineError {
        code: descriptor.code,
        message,
        retryable: descriptor.retryable,
        hint: descriptor.hint,
    };
    let command = command.unwrap_or("unknown");
    let mut safe_options = options.clone();
    safe_options.request_id = valid_request_id(options);
    match options.output {
        OutputFormat::Json => {
            let envelope = Envelope {
                api_version: API_VERSION,
                command: command.to_owned(),
                request_id: safe_options.request_id.clone(),
                workspace: None,
                exit_code: 2,
                ok: false,
                data: Value::Null,
                error: Some(machine_error),
            };
            println!("{}", serde_json::to_string(&envelope)?);
        }
        OutputFormat::Jsonl => {
            emit_event_with_workspace(
                &safe_options,
                "error",
                command,
                None,
                Some(2),
                Some(false),
                serde_json::to_value(machine_error)?,
            )?;
        }
        OutputFormat::Text => {}
    }
    Ok(())
}

fn workspace_context(root: Option<&Path>) -> Option<WorkspaceContext> {
    root.map(|root| WorkspaceContext {
        path: root.to_string_lossy().into_owned(),
        revision: crate::workspace::revision(root).ok(),
    })
}

fn workspace_context_with_revision(
    root: Option<&Path>,
    revision: Option<&str>,
) -> Option<WorkspaceContext> {
    root.map(|root| WorkspaceContext {
        path: root.to_string_lossy().into_owned(),
        revision: revision.map(str::to_owned),
    })
}

/// Do not reflect an invalid preflight request ID in an error document.
fn valid_request_id(options: &AutomationOptions) -> Option<String> {
    options
        .request_id
        .as_deref()
        .filter(|request_id| validate_request_id(request_id).is_ok())
        .map(str::to_owned)
}

struct ErrorDescriptor {
    code: &'static str,
    retryable: bool,
    hint: Option<&'static str>,
}

fn error_descriptor(message: &str) -> ErrorDescriptor {
    let lower = message.to_ascii_lowercase();
    if lower.contains("workspace revision changed") {
        ErrorDescriptor {
            code: "stale_workspace_revision",
            retryable: true,
            hint: Some("Run the plan again and apply the new workspace revision."),
        }
    } else if is_invalid_argument_message(&lower) {
        ErrorDescriptor {
            code: "invalid_arguments",
            retryable: false,
            hint: None,
        }
    } else if lower.contains("unknown repository selector") {
        ErrorDescriptor {
            code: "unknown_repository",
            retryable: false,
            hint: Some("Use list --output json to inspect canonical repository names."),
        }
    } else if lower.contains("ambiguous repository selector") {
        ErrorDescriptor {
            code: "ambiguous_repository",
            retryable: false,
            hint: Some("Use a canonical repository name or workspace-relative directory."),
        }
    } else if lower.contains("pattern matched nothing") {
        ErrorDescriptor {
            code: "selector_no_match",
            retryable: false,
            hint: Some("Inspect names with list --output json before retrying."),
        }
    } else if lower.contains("no workspace.toml found") {
        ErrorDescriptor {
            code: "workspace_not_found",
            retryable: false,
            hint: Some("Run scan or set BATCH_GIT_WORKSPACE to an existing workspace."),
        }
    } else if lower.contains("workspace.toml")
        && (lower.contains("parse") || lower.contains("invalid") || lower.contains("validate"))
    {
        ErrorDescriptor {
            code: "workspace_manifest_invalid",
            retryable: false,
            hint: Some("Fix workspace.toml and retry."),
        }
    } else if lower.contains("timed out") {
        ErrorDescriptor {
            code: "timeout",
            retryable: true,
            hint: Some("Retry with a larger --timeout after checking connectivity."),
        }
    } else if lower.contains("lock") {
        ErrorDescriptor {
            code: "workspace_locked",
            retryable: true,
            hint: Some("Wait for the other batch-git operation to finish and retry."),
        }
    } else if lower.contains("schedule") {
        ErrorDescriptor {
            code: "schedule_invalid",
            retryable: false,
            hint: None,
        }
    } else {
        ErrorDescriptor {
            code: "operation_failed",
            retryable: false,
            hint: None,
        }
    }
}

/// Recognize validation messages emitted before an operation begins.
///
/// The CLI intentionally keeps `anyhow` context for human diagnostics, so this small boundary
/// classifier covers every locally-validated global option rather than exposing unstable prose to
/// automation consumers.
fn is_invalid_argument_message(message: &str) -> bool {
    [
        "invalid arguments",
        "unexpected argument",
        "invalid timeout:",
        "timeout must be",
        "timeout unit must be",
        "timeout is too large",
        "request id must contain",
        "invalid --jobs value",
        "jobs must be at least 1",
        "--jobs requires a value",
        "--output requires a value",
        "--timeout requires a value",
        "--request-id requires a value",
        "--expect-workspace-revision requires a value",
        "invalid output format:",
        "--plan is only valid",
        "--plan conflicts with --apply",
        "--apply is only valid",
        "--apply requires --expect-workspace-revision",
        "--expect-workspace-revision requires --apply",
        "--all cannot be combined with repository selectors or --match",
        "commit message cannot be empty",
        "git passthrough requires arguments after --",
        "use schedule plan ",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

/// Remove URL user-info before including a diagnostic in a machine-readable protocol.
///
/// The operation protocol intentionally excludes child stdout/stderr. This is a final guard for
/// top-level errors, which can still contain an HTTP(S) URL from an underlying library.
pub(crate) fn sanitize_message(message: &str) -> String {
    let mut result = String::with_capacity(message.len());
    let mut remainder = message;
    while let Some(scheme_index) = remainder.find("://") {
        let (prefix, after_prefix) = remainder.split_at(scheme_index + 3);
        result.push_str(prefix);
        let end = after_prefix
            .find(|character: char| {
                character.is_whitespace() || matches!(character, '/' | '\\' | '"' | '\'' | ')')
            })
            .unwrap_or(after_prefix.len());
        let (authority, tail) = after_prefix.split_at(end);
        if let Some((_, host)) = authority.rsplit_once('@') {
            result.push_str("***@");
            result.push_str(host);
        } else {
            result.push_str(authority);
        }
        remainder = tail;
    }
    result.push_str(remainder);
    result
}
