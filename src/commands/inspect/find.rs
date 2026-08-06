//! Local and remote-tracking branch discovery without network access.

use std::collections::HashSet;

use anyhow::Result;
use serde::Serialize;

use crate::automation::{self, AutomationOptions};
use crate::cli::FindArgs;
use crate::git::{self, BranchKind};
use crate::parallel::map_ordered;
use crate::selector::wildcard_matches;
use crate::{color, settings, table, workspace};

#[derive(Serialize)]
struct FindOutput<'a> {
    pattern: &'a str,
    repository_pattern: Option<&'a str>,
    repositories_scanned: usize,
    unavailable: usize,
    branches: Vec<FindBranch>,
}

#[derive(Serialize)]
struct FindBranch {
    repository: String,
    directory: String,
    name: String,
    #[serde(skip)]
    default_branch: String,
    kind: &'static str,
    remote: Option<String>,
    is_current: bool,
    commit: String,
    commit_short: String,
    commit_time: i64,
}

/// Search already available refs without implicitly accessing a remote.
pub(in crate::commands) fn find(
    arguments: FindArgs,
    jobs: usize,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let repositories = manifest
        .repositories
        .iter()
        .filter(|repository| {
            arguments
                .repo
                .as_deref()
                .is_none_or(|pattern| wildcard_matches(pattern, &repository.name))
        })
        .cloned()
        .collect::<Vec<_>>();
    let include_local = !arguments.remote;
    let include_remote = !arguments.local;
    let scans = map_ordered(&repositories, jobs, |repository| {
        let path = root.join(&repository.directory);
        if !git::is_repository(&path) {
            return (repository.clone(), None);
        }
        let branches = git::branches(&path, include_local, include_remote)
            .ok()
            .map(|branches| {
                branches
                    .into_iter()
                    .filter(|branch| wildcard_matches(&arguments.pattern, &branch.name))
                    .collect::<Vec<_>>()
            });
        (repository.clone(), branches)
    })?;

    let unavailable = scans
        .iter()
        .filter(|(_, branches)| branches.is_none())
        .count();
    let mut matches = Vec::new();
    for (repository, branches) in scans {
        for branch in branches.unwrap_or_default() {
            matches.push(FindBranch {
                repository: repository.name.clone(),
                directory: repository.directory.clone(),
                name: branch.name,
                default_branch: repository.default_branch.clone(),
                kind: branch.kind.label(),
                remote: branch.remote,
                is_current: branch.is_current,
                commit: branch.commit,
                commit_short: branch.commit_short,
                commit_time: branch.commit_time,
            });
        }
    }
    let repositories_with_matches = matches
        .iter()
        .map(|branch| branch.repository.as_str())
        .collect::<HashSet<_>>()
        .len();
    let local = matches
        .iter()
        .filter(|branch| branch.kind == BranchKind::Local.label())
        .count();
    let remote = matches.len() - local;

    let output = FindOutput {
        pattern: &arguments.pattern,
        repository_pattern: arguments.repo.as_deref(),
        repositories_scanned: repositories.len(),
        unavailable,
        branches: matches,
    };
    let exit_code = i32::from(unavailable > 0);
    if automation.is_machine() {
        automation::emit_data(automation, "find", Some(&root), exit_code, &output)?;
    } else if arguments.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        let feature_branch = settings::current_feature_branch()?;
        let rows = output
            .branches
            .iter()
            .map(|branch| {
                vec![
                    branch.repository.clone(),
                    branch.kind.to_owned(),
                    branch.remote.clone().unwrap_or_else(|| "-".to_owned()),
                    if branch.is_current {
                        color::green("yes")
                    } else {
                        "no".to_owned()
                    },
                    color::branch(
                        &branch.name,
                        &branch.default_branch,
                        feature_branch.as_deref(),
                    ),
                ]
            })
            .collect::<Vec<_>>();
        print!(
            "{}",
            table::render(
                &["REPOSITORY", "KIND", "REMOTE", "CURRENT", "BRANCH"],
                &rows
            )
        );
        println!();
        let unavailable_summary = if unavailable == 0 {
            String::new()
        } else {
            format!("; {} unavailable", color::red(unavailable))
        };
        println!(
            "summary: {} {} in {repositories_with_matches} {}; {local} local, {remote} remote{unavailable_summary}",
            output.branches.len(),
            if output.branches.len() == 1 {
                "branch"
            } else {
                "branches"
            },
            if repositories_with_matches == 1 {
                "repository"
            } else {
                "repositories"
            },
        );
    }
    Ok(exit_code)
}
