//! Manifest repository listing with lightweight branch inspection.

use anyhow::Result;
use serde::Serialize;

use crate::automation::{self, AutomationOptions};
use crate::cli::ListArgs;
use crate::{color, git, settings, table, workspace};

#[derive(Serialize)]
struct ListOutput<'a> {
    workspace: String,
    jobs: usize,
    repositories: Vec<ListRepository<'a>>,
}

#[derive(Serialize)]
struct ListRepository<'a> {
    name: &'a str,
    directory: &'a str,
    default_branch: &'a str,
    current_branch: Option<String>,
    materialized: bool,
    synced_at: Option<&'a str>,
}

/// List registered repositories and their live current branch.
pub(in crate::commands) fn list(
    arguments: ListArgs,
    jobs: usize,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let repositories: Vec<ListRepository<'_>> = manifest
        .repositories
        .iter()
        .map(|repository| {
            let path = workspace::repository_path(&root, &repository.directory)?;
            let materialized = git::is_repository(&path);
            Ok(ListRepository {
                name: &repository.name,
                directory: &repository.directory,
                default_branch: &repository.default_branch,
                current_branch: materialized
                    .then(|| git::current_branch_summary(&path).ok())
                    .flatten(),
                materialized,
                synced_at: repository.synced_at.as_deref(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let output = ListOutput {
        workspace: root.to_string_lossy().into_owned(),
        jobs,
        repositories,
    };
    if automation.is_machine() {
        automation::emit_data(automation, "list", Some(&root), 0, &output)?;
    } else if arguments.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        let feature_branch = settings::current_feature_branch()?;
        let rows = output
            .repositories
            .iter()
            .map(|repository| {
                let state = if repository.materialized {
                    repository
                        .current_branch
                        .clone()
                        .unwrap_or_else(|| color::yellow("unknown"))
                } else {
                    color::red("missing")
                };
                vec![
                    repository.name.to_owned(),
                    color::branch(&state, repository.default_branch, feature_branch.as_deref()),
                    color::blue(repository.default_branch),
                ]
            })
            .collect::<Vec<_>>();
        print!("{}", table::render(&["NAME", "CURRENT", "DEFAULT"], &rows));
        println!();
        println!("{} repositories", manifest.repositories.len());
    }
    Ok(0)
}
