//! v1 JSON envelope and JSONL lifecycle serialization.

use std::path::Path;

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use super::error::{error_descriptor, sanitize_message};
use super::options::{AutomationOptions, OutputFormat, validate_request_id};

/// Stable protocol version emitted by `--output json` and `--output jsonl`.
pub(crate) const API_VERSION: &str = "v1";

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

/// Emit a response using the manifest revision captured with the planned selection.
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

/// Emit one JSONL event. This is a no-op for other output formats.
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

/// Emit the aggregate final JSONL event.
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

/// Emit a top-level failure without contaminating machine stdout with text diagnostics.
pub(crate) fn emit_error(
    options: &AutomationOptions,
    command: Option<&str>,
    error: &anyhow::Error,
) -> Result<()> {
    let message = sanitize_message(&format!("{error:#}"));
    let descriptor = error_descriptor(error);
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
