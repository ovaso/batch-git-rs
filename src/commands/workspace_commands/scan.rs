//! Local repository discovery and incremental manifest population.

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::Result;
use serde_json::json;

use super::super::verify_apply_revision;
use super::support::{relative_string, unique_name};
use crate::automation::{self, AutomationOptions};
use crate::cli::ScanArgs;
use crate::model::{RepositoryRecord, now};
use crate::parallel::map_ordered;
use crate::report::JsonlProgress;
use crate::workspace::WorkspaceLock;
use crate::{git, settings, workspace};

/// Discover existing repositories and append new declarations without removing old ones.
pub(in crate::commands) fn scan(
    arguments: ScanArgs,
    jobs: usize,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::current_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let mut manifest = workspace::read_or_new(&root)?;
    let depth = settings::scan_depth(arguments.depth)?;
    let paths = git::discover(&root, depth)?;
    let known_directories = manifest
        .repositories
        .iter()
        .map(|repository| repository.directory.clone())
        .collect::<HashSet<_>>();
    let candidates = paths
        .into_iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(&root).ok()?;
            let directory = relative_string(relative).ok()?;
            (!known_directories.contains(&directory)).then_some((path, directory))
        })
        .collect::<Vec<(PathBuf, String)>>();

    let jsonl_progress = JsonlProgress::new(automation, "scan", &root, candidates.len())?;
    let inspected = map_ordered(&candidates, jobs, |(path, directory)| {
        (path.clone(), directory.clone(), git::inspect(path))
    })?;
    let mut succeeded = 0;
    let mut failures = 0;
    let mut results = Vec::new();
    for (index, (path, directory, inspection)) in inspected.into_iter().enumerate() {
        let result = match inspection {
            Ok(info) => {
                let base_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("repository");
                let name = unique_name(base_name, &directory, &manifest.repositories);
                manifest.repositories.push(RepositoryRecord {
                    name: name.clone(),
                    directory: directory.clone(),
                    default_branch: info.default_branch,
                    primary_remote: info.primary_remote,
                    remotes: info.remotes,
                    created_at: now(),
                    synced_at: None,
                });
                let result = json!({
                    "repository": name,
                    "directory": directory,
                    "status": "ok",
                    "detail": "repository registered",
                    "synchronized": false,
                });
                succeeded += 1;
                if !automation.is_machine() {
                    println!(
                        "added {} ({})",
                        result["repository"].as_str().unwrap_or("-"),
                        result["directory"].as_str().unwrap_or("-")
                    );
                }
                result
            }
            Err(error) => {
                failures += 1;
                let result = json!({
                    "repository": directory,
                    "directory": directory,
                    "status": "failed",
                    "reason_code": "inspection_failed",
                    "detail": automation::sanitize_message(&error.to_string()),
                    "synchronized": false,
                });
                if !automation.is_machine() {
                    eprintln!("failed {}: {error:#}", path.display());
                }
                result
            }
        };
        if let Some(progress) = &jsonl_progress {
            progress.repository_finished_value(index, result.clone());
        }
        results.push(result);
    }
    workspace::write(&root, &mut manifest)?;
    let exit_code = i32::from(failures > 0);
    if automation.is_machine() {
        let data = json!({
            "summary": {"ok": succeeded, "skipped": 0, "failed": failures},
            "results": results,
            "registered_repositories": manifest.repositories.len(),
        });
        if automation.output == automation::OutputFormat::Jsonl {
            automation::emit_finished(automation, "scan", Some(&root), exit_code, data)?;
        } else {
            automation::emit_data(automation, "scan", Some(&root), exit_code, &data)?;
        }
    } else {
        println!(
            "workspace: {} repositories, {} newly added, {} failed",
            manifest.repositories.len(),
            candidates.len().saturating_sub(failures),
            failures
        );
    }
    Ok(exit_code)
}
