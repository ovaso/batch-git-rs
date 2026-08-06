//! Workspace-wide working tree and upstream status aggregation.

use anyhow::Result;
use serde_json::json;

use crate::automation::{self, AutomationOptions};
use crate::cli::MachineReadableArgs;
use crate::parallel::map_ordered;
use crate::{color, git, settings, table, workspace};

/// Higher-level workspace state classification independent of raw Git status flags.
#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkspaceStatusKind {
    Clean,
    Dirty,
    Conflict,
    Missing,
    NotGit,
    Error,
}

impl WorkspaceStatusKind {
    fn label(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Dirty => "dirty",
            Self::Conflict => "conflict",
            Self::Missing => "missing",
            Self::NotGit => "not-git",
            Self::Error => "error",
        }
    }

    fn unavailable(self) -> bool {
        matches!(self, Self::Missing | Self::NotGit | Self::Error)
    }

    fn colored_label(self) -> String {
        match self {
            Self::Clean => color::green(self.label()),
            Self::Dirty => color::yellow(self.label()),
            Self::Conflict | Self::Missing | Self::NotGit | Self::Error => color::red(self.label()),
        }
    }

    fn colored_count(self, count: usize) -> String {
        match self {
            Self::Clean => color::green(count),
            Self::Dirty => color::yellow(count),
            Self::Conflict | Self::Missing | Self::NotGit | Self::Error => color::red(count),
        }
    }
}

struct WorkspaceStatusRow {
    repository: String,
    branch: String,
    default_branch: String,
    kind: WorkspaceStatusKind,
    changes: String,
    upstream: String,
    changed_paths: usize,
}

/// Read repository statuses concurrently and emit them in manifest order.
pub(in crate::commands) fn status(
    arguments: MachineReadableArgs,
    jobs: usize,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let feature_branch = settings::current_feature_branch()?;
    let statuses = map_ordered(&manifest.repositories, jobs, |repository| {
        let path = root.join(&repository.directory);
        if !path.exists() {
            return WorkspaceStatusRow {
                repository: repository.name.clone(),
                branch: "-".to_owned(),
                default_branch: repository.default_branch.clone(),
                kind: WorkspaceStatusKind::Missing,
                changes: "-".to_owned(),
                upstream: "-".to_owned(),
                changed_paths: 0,
            };
        }
        if !git::is_repository(&path) {
            return WorkspaceStatusRow {
                repository: repository.name.clone(),
                branch: "-".to_owned(),
                default_branch: repository.default_branch.clone(),
                kind: WorkspaceStatusKind::NotGit,
                changes: "-".to_owned(),
                upstream: "-".to_owned(),
                changed_paths: 0,
            };
        }
        match git::status_summary(&path) {
            Ok(status) => {
                let kind = if status.changes.conflicted > 0 {
                    WorkspaceStatusKind::Conflict
                } else if status.changes.total() > 0 {
                    WorkspaceStatusKind::Dirty
                } else {
                    WorkspaceStatusKind::Clean
                };
                WorkspaceStatusRow {
                    repository: repository.name.clone(),
                    branch: status.branch,
                    default_branch: repository.default_branch.clone(),
                    kind,
                    changes: status.changes.compact(),
                    upstream: status.upstream.label(),
                    changed_paths: status.changes.total(),
                }
            }
            Err(_) => WorkspaceStatusRow {
                repository: repository.name.clone(),
                branch: git::current_branch_summary(&path)
                    .unwrap_or_else(|_| "(unknown)".to_owned()),
                default_branch: repository.default_branch.clone(),
                kind: WorkspaceStatusKind::Error,
                changes: "-".to_owned(),
                upstream: "-".to_owned(),
                changed_paths: 0,
            },
        }
    })?;
    let exit_code = i32::from(statuses.iter().any(|status| status.kind.unavailable()));
    let machine_data = || {
        statuses
            .iter()
            .map(|status| {
                json!({
                    "repository": status.repository,
                    "branch": status.branch,
                    "default_branch": status.default_branch,
                    "state": status.kind.label(),
                    "changes": status.changes,
                    "changed_paths": status.changed_paths,
                    "upstream": status.upstream
                })
            })
            .collect::<Vec<_>>()
    };
    if automation.is_machine() {
        automation::emit_data(
            automation,
            "status",
            Some(&root),
            exit_code,
            &json!({"repositories": machine_data()}),
        )?;
        return Ok(exit_code);
    }
    if arguments.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"repositories": machine_data()}))?
        );
        return Ok(exit_code);
    }
    let rows = statuses
        .iter()
        .map(|status| {
            vec![
                status.repository.clone(),
                status.kind.colored_label(),
                status.changes.clone(),
                status.upstream.clone(),
                color::branch(
                    &status.branch,
                    &status.default_branch,
                    feature_branch.as_deref(),
                ),
            ]
        })
        .collect::<Vec<_>>();
    print!(
        "{}",
        table::render(
            &["REPOSITORY", "STATE", "CHANGES", "UPSTREAM", "BRANCH"],
            &rows
        )
    );

    let state_counts = [
        (WorkspaceStatusKind::Clean, "clean"),
        (WorkspaceStatusKind::Dirty, "dirty"),
        (WorkspaceStatusKind::Conflict, "conflict"),
        (WorkspaceStatusKind::Missing, "missing"),
        (WorkspaceStatusKind::NotGit, "not-git"),
        (WorkspaceStatusKind::Error, "error"),
    ]
    .into_iter()
    .filter_map(|(kind, label)| {
        let count = statuses.iter().filter(|status| status.kind == kind).count();
        (count > 0).then(|| format!("{} {label}", kind.colored_count(count)))
    })
    .collect::<Vec<_>>()
    .join(", ");
    let changed_paths = statuses
        .iter()
        .map(|status| status.changed_paths)
        .sum::<usize>();
    let state_summary = if state_counts.is_empty() {
        String::new()
    } else {
        format!(", {state_counts}")
    };
    println!();
    let changed_paths = if changed_paths > 0 {
        color::yellow(changed_paths)
    } else {
        changed_paths.to_string()
    };
    println!(
        "summary: {} repositories{state_summary}; {changed_paths} changed paths",
        statuses.len()
    );
    Ok(exit_code)
}
