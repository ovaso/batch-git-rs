//! Fast current-branch summary without working tree or remote inspection.

use anyhow::Result;
use serde_json::json;

use crate::automation::{self, AutomationOptions};
use crate::cli::MachineReadableArgs;
use crate::parallel::map_ordered;
use crate::{color, git, settings, table, workspace};

pub(in crate::commands) fn branch(
    arguments: MachineReadableArgs,
    jobs: usize,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let feature_branch = settings::current_feature_branch()?;
    let states = map_ordered(&manifest.repositories, jobs, |repository| {
        let path = root.join(&repository.directory);
        let state = if !path.exists() {
            "(missing)".to_owned()
        } else if !git::is_repository(&path) {
            "(not-git)".to_owned()
        } else {
            git::current_branch_summary(&path).unwrap_or_else(|_| "(unknown)".to_owned())
        };
        (
            repository.name.clone(),
            repository.default_branch.clone(),
            state,
        )
    })?;
    if automation.is_machine() {
        let unavailable = states
            .iter()
            .filter(|(_, _, branch)| branch.starts_with('('))
            .count();
        let exit_code = i32::from(unavailable > 0);
        let repositories = states
            .iter()
            .map(|(name, default_branch, branch)| {
                json!({"repository": name, "default_branch": default_branch, "branch": branch})
            })
            .collect::<Vec<_>>();
        automation::emit_data(
            automation,
            "branch",
            Some(&root),
            exit_code,
            &json!({"repositories": repositories}),
        )?;
        return Ok(exit_code);
    }
    if arguments.json {
        let unavailable = states
            .iter()
            .filter(|(_, _, branch)| branch.starts_with('('))
            .count();
        let repositories = states
            .iter()
            .map(|(name, default_branch, branch)| {
                json!({"repository": name, "default_branch": default_branch, "branch": branch})
            })
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"repositories": repositories}))?
        );
        return Ok(i32::from(unavailable > 0));
    }
    let rows = states
        .into_iter()
        .map(|(name, default_branch, state)| {
            vec![
                name,
                color::branch(&state, &default_branch, feature_branch.as_deref()),
            ]
        })
        .collect::<Vec<_>>();
    print!("{}", table::render(&["REPOSITORY", "BRANCH"], &rows));
    Ok(0)
}
