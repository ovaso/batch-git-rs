//! Single repository clone and manifest registration.

use std::path::PathBuf;

use anyhow::{Result, bail};
use serde_json::json;

use super::super::{git_execution_options, verify_apply_revision};
use super::support::{relative_string, reserve_clone_destination, unique_name};
use crate::automation::{self, AutomationOptions};
use crate::cli::CloneArgs;
use crate::git::{self, CloneOptions};
use crate::model::{RepositoryRecord, WORKSPACE_FILE, now, validate_directory};
use crate::report::JsonlProgress;
use crate::workspace::{self, WorkspaceLock};

/// Clone one repository and register it only after cloning and inspection succeed.
pub(in crate::commands) fn clone_repository(
    arguments: CloneArgs,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::find_root_optional()?.unwrap_or(workspace::current_root()?);
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let mut manifest = workspace::read_or_new(&root)?;
    let directory = arguments
        .directory
        .unwrap_or_else(|| PathBuf::from(default_clone_directory(&arguments.repository)));
    let directory_string = relative_string(&directory)?;
    validate_directory(&directory_string)?;
    if manifest
        .repositories
        .iter()
        .any(|repository| repository.directory == directory_string)
    {
        bail!("repository directory is already registered: {directory_string}");
    }

    let target = root.join(&directory);
    if let Some(depth) = arguments.depth
        && depth == 0
    {
        bail!("--depth must be at least 1");
    }
    reserve_clone_destination(&target)?;
    let jsonl_progress = JsonlProgress::new(automation, "clone", &root, 1)?;
    if let Err(error) = git::clone_repository_with_options(
        &arguments.repository,
        &target,
        CloneOptions {
            remote_name: "origin",
            branch: arguments.branch.as_deref(),
            depth: arguments.depth,
            single_branch: arguments.single_branch,
            progress: None,
            allow_stdin: true,
        },
        git_execution_options(automation, true),
    ) {
        if automation.is_machine() {
            let detail = automation::sanitize_message(&error.to_string());
            let reason_code = if detail.to_ascii_lowercase().contains("timed out") {
                "timeout"
            } else {
                "git_failed"
            };
            let result = json!({
                "repository": directory_string,
                "directory": directory_string,
                "status": "failed",
                "reason_code": reason_code,
                "detail": detail,
                "synchronized": false,
            });
            if let Some(progress) = &jsonl_progress {
                progress.repository_finished_value(0, result.clone());
            }
            if automation.output == automation::OutputFormat::Jsonl {
                automation::emit_finished(
                    automation,
                    "clone",
                    Some(&root),
                    1,
                    json!({"result": result}),
                )?;
            } else {
                automation::emit_data(
                    automation,
                    "clone",
                    Some(&root),
                    1,
                    &json!({"result": result}),
                )?;
            }
        } else {
            eprintln!("clone failed: {error:#}");
        }
        return Ok(1);
    }

    let info = git::inspect(&target)?;
    let name = unique_name(
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("repository"),
        &directory_string,
        &manifest.repositories,
    );
    let timestamp = now();
    manifest.repositories.push(RepositoryRecord {
        name: name.clone(),
        directory: directory_string.clone(),
        default_branch: info.default_branch,
        primary_remote: info.primary_remote,
        remotes: info.remotes,
        created_at: timestamp.clone(),
        synced_at: Some(timestamp),
    });
    workspace::write(&root, &mut manifest)?;
    if automation.is_machine() {
        let result = json!({
            "repository": name,
            "directory": directory_string,
            "status": "ok",
            "detail": "repository cloned and registered",
            "synchronized": true,
        });
        if let Some(progress) = &jsonl_progress {
            progress.repository_finished_value(0, result.clone());
        }
        if automation.output == automation::OutputFormat::Jsonl {
            automation::emit_finished(
                automation,
                "clone",
                Some(&root),
                0,
                json!({"result": result}),
            )?;
        } else {
            automation::emit_data(
                automation,
                "clone",
                Some(&root),
                0,
                &json!({"result": result}),
            )?;
        }
    } else {
        println!(
            "registered {name} in {}",
            root.join(WORKSPACE_FILE).display()
        );
    }
    Ok(0)
}

/// Derive the default clone directory from the final URL or path segment.
pub(in crate::commands) fn default_clone_directory(repository: &str) -> String {
    let trimmed = repository.trim_end_matches('/').trim_end_matches(".git");
    trimmed
        .rsplit(['/', ':'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("repository")
        .to_owned()
}
