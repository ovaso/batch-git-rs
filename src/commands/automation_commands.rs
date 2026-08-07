//! Automation capability, environment, and schema discovery commands.

use anyhow::Result;
use serde::Serialize;
use serde_json::json;

use crate::automation::{self, AutomationOptions};
use crate::cli::{
    EnvArgs, EnvCommand, EnvListArgs, SchemaArgs, SchemaDocument, command_capabilities,
};
use crate::{color, settings, table};

/// Return the current binary's protocol surface rather than requiring agents to guess it.
pub(super) fn capabilities(automation: &AutomationOptions) -> Result<i32> {
    let data = json!({
        "binary_version": env!("CARGO_PKG_VERSION"),
        "automation_protocol": automation::API_VERSION,
        "output_formats": ["text", "json", "jsonl"],
        "schemas": ["operation-result", "workspace"],
        "commands": command_capabilities(),
        "automation": {
            "structured_top_level_errors": true,
            "per_repository_results": true,
            "jsonl_events": true,
            "plan": {
                "supported": true,
                "apply_requires_workspace_revision": true,
                "repository_state_preconditions": false
            },
            "non_interactive": true,
            "child_process_timeout": true
        },
        "safety": {
            "atomicity": "per_repository",
            "implicit_merge_rebase_stash_reset_clean_force_push": false,
            "commit_stages_content": false,
            "add_rejects_unresolved_conflicts": true,
            "commit_rejects_repository_operations": true,
            "unstage_preserves_working_trees": true,
            "passthrough_risk": "unclassified"
        }
    });
    if automation.is_machine() {
        automation::emit_data(automation, "capabilities", None, 0, &data)?;
    } else {
        println!("batch-git {}", env!("CARGO_PKG_VERSION"));
        println!("automation protocol: {}", automation::API_VERSION);
        println!("machine output: json, jsonl");
        println!("schemas: operation-result, workspace");
        println!("plan/apply: workspace revision precondition");
    }
    Ok(0)
}

/// Show supported environment variables without requiring a workspace manifest.
pub(super) fn environment(
    arguments: EnvArgs,
    jobs: usize,
    automation: &AutomationOptions,
) -> Result<i32> {
    match arguments.command {
        EnvCommand::List(arguments) => environment_list(arguments, jobs, automation),
    }
}

fn environment_list(
    arguments: EnvListArgs,
    jobs: usize,
    automation: &AutomationOptions,
) -> Result<i32> {
    let variables = settings::environment_variables(jobs)?;
    if automation.is_machine() {
        let variables = variables
            .iter()
            .map(|variable| EnvironmentVariableOutput {
                name: variable.name,
                description: arguments.description.then_some(variable.description),
                default: &variable.default,
                current: &variable.current,
            })
            .collect::<Vec<_>>();
        automation::emit_data(
            automation,
            "env list",
            None,
            0,
            &json!({"variables": variables}),
        )?;
    } else {
        let (headers, rows) = if arguments.description {
            (
                vec!["VARIABLE", "DESCRIPTION", "DEFAULT", "CURRENT"],
                variables
                    .iter()
                    .map(|variable| {
                        vec![
                            variable.name.to_owned(),
                            variable.description.to_owned(),
                            variable.default.clone(),
                            color::green(&variable.current),
                        ]
                    })
                    .collect::<Vec<_>>(),
            )
        } else {
            (
                vec!["VARIABLE", "DEFAULT", "CURRENT"],
                variables
                    .iter()
                    .map(|variable| {
                        vec![
                            variable.name.to_owned(),
                            variable.default.clone(),
                            color::green(&variable.current),
                        ]
                    })
                    .collect::<Vec<_>>(),
            )
        };
        print!("{}", table::render(&headers, &rows));
    }
    Ok(0)
}

#[derive(Serialize)]
struct EnvironmentVariableOutput<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    default: &'a str,
    current: &'a str,
}

/// Print a JSON Schema document. Text mode prints the schema itself for shell-friendly use;
/// `--output json` wraps it in the normal v1 receipt.
pub(super) fn schema(arguments: SchemaArgs, automation: &AutomationOptions) -> Result<i32> {
    let document = match arguments.document {
        SchemaDocument::OperationResult => operation_result_schema(),
        SchemaDocument::Workspace => workspace_schema(),
    };
    if automation.is_machine() {
        automation::emit_data(automation, "schema", None, 0, &document)?;
    } else {
        println!("{}", serde_json::to_string_pretty(&document)?);
    }
    Ok(0)
}

fn operation_result_schema() -> serde_json::Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://github.com/ovaso/batch-git-rs/schemas/operation-result-v1.json",
        "title": "batch-git automation result v1",
        "type": "object",
        "required": ["api_version", "command", "exit_code", "ok", "data", "error"],
        "properties": {
            "api_version": {"const": automation::API_VERSION},
            "command": {"type": "string", "minLength": 1},
            "request_id": {"type": "string", "minLength": 1, "maxLength": 128},
            "workspace": {
                "type": "object",
                "required": ["path"],
                "properties": {"path": {"type": "string"}, "revision": {"type": "string"}},
                "additionalProperties": true
            },
            "exit_code": {"type": "integer", "minimum": 0, "maximum": 255},
            "ok": {"type": "boolean"},
            "data": {},
            "error": {
                "anyOf": [
                    {"type": "null"},
                    {
                        "type": "object",
                        "required": ["code", "message", "retryable"],
                        "properties": {
                            "code": {"type": "string"},
                            "message": {"type": "string"},
                            "retryable": {"type": "boolean"},
                            "hint": {"type": "string"}
                        },
                        "additionalProperties": true
                    }
                ]
            }
        },
        "additionalProperties": true
    })
}

fn workspace_schema() -> serde_json::Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://github.com/ovaso/batch-git-rs/schemas/workspace-v1.json",
        "title": "workspace.toml v1 JSON representation",
        "type": "object",
        "required": ["version", "created_at", "updated_at"],
        "properties": {
            "version": {"const": 1},
            "created_at": {"type": "string"},
            "updated_at": {"type": "string"},
            "repositories": {"type": "array", "items": {"$ref": "#/$defs/repository"}},
            "schedules": {"type": "array", "items": {"$ref": "#/$defs/schedule"}}
        },
        "$defs": {
            "repository": {
                "type": "object",
                "required": ["name", "directory", "default_branch", "created_at"],
                "properties": {
                    "name": {"type": "string"}, "directory": {"type": "string"},
                    "default_branch": {"type": "string"}, "primary_remote": {"type": "string"},
                    "remotes": {"type": "array", "items": {"type": "object"}},
                    "created_at": {"type": "string"}, "synced_at": {"type": ["string", "null"]}
                },
                "additionalProperties": true
            },
            "schedule": {
                "type": "object",
                "required": ["name", "scope"],
                "properties": {
                    "name": {"type": "string"}, "enabled": {"type": "boolean"},
                    "action": {"enum": ["sync", "pull"]}, "timezone": {"type": "string"},
                    "overlap": {"enum": ["skip", "queue"]}, "scope": {"type": "object"}
                },
                "additionalProperties": true
            }
        },
        "additionalProperties": true
    })
}
