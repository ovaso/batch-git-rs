//! Workspace and single-repository metadata inspection.

use anyhow::Result;
use serde::Serialize;

use crate::automation::{self, AutomationOptions};
use crate::cli::InfoArgs;
use crate::error::{ErrorCode, classified};
use crate::git::{self, RepositoryRuntimeInfo, RepositoryRuntimeState};
use crate::model::WORKSPACE_FILE;
use crate::parallel::map_ordered;
use crate::{color, settings, table, workspace};

#[derive(Serialize)]
struct WorkspaceInfoOutput {
    root: String,
    manifest: String,
    version: u32,
    created_at: String,
    updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_feature_branch: Option<String>,
    jobs: usize,
    repositories: WorkspaceRepositoryCounts,
}

#[derive(Serialize)]
struct WorkspaceRepositoryCounts {
    total: usize,
    materialized: usize,
    missing: usize,
    not_git: usize,
    bare: usize,
    error: usize,
}

#[derive(Serialize)]
struct RepositoryInfoOutput {
    name: String,
    directory: String,
    path: String,
    state: &'static str,
    current_branch: Option<String>,
    default_branch: String,
    head: Option<String>,
    primary_remote: String,
    local_branches: Option<usize>,
    remote_branches: Option<usize>,
    created_at: String,
    synced_at: Option<String>,
    remotes: Vec<InfoRemote>,
}

#[derive(Serialize)]
struct InfoRemote {
    name: String,
    primary: bool,
    fetch_url: String,
    push_url: Option<String>,
}

/// Show workspace-level or one repository's manifest and live Git metadata.
pub(in crate::commands) fn info(
    arguments: InfoArgs,
    jobs: usize,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let current_feature_branch = settings::current_feature_branch()?;
    let Some(selector) = arguments.repository.as_deref() else {
        let runtime =
            map_ordered(
                &manifest.repositories,
                jobs,
                |repository| match workspace::repository_path(&root, &repository.directory) {
                    Ok(path) => git::repository_runtime_info(&path),
                    Err(_) => RepositoryRuntimeInfo {
                        state: RepositoryRuntimeState::Error,
                        current_branch: None,
                        head: None,
                        local_branches: None,
                        remote_branches: None,
                    },
                },
            )?;
        let count = |state| {
            runtime
                .iter()
                .filter(|repository| repository.state == state)
                .count()
        };
        let output = WorkspaceInfoOutput {
            root: root.to_string_lossy().into_owned(),
            manifest: root.join(WORKSPACE_FILE).to_string_lossy().into_owned(),
            version: manifest.version,
            created_at: manifest.created_at.clone(),
            updated_at: manifest.updated_at.clone(),
            current_feature_branch,
            jobs,
            repositories: WorkspaceRepositoryCounts {
                total: manifest.repositories.len(),
                materialized: count(RepositoryRuntimeState::Available),
                missing: count(RepositoryRuntimeState::Missing),
                not_git: count(RepositoryRuntimeState::NotGit),
                bare: count(RepositoryRuntimeState::Bare),
                error: count(RepositoryRuntimeState::Error),
            },
        };
        if automation.is_machine() {
            automation::emit_data(automation, "info", Some(&root), 0, &output)?;
        } else if arguments.json {
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            print_workspace_info(&output);
        }
        return Ok(0);
    };

    let matches = manifest
        .repositories
        .iter()
        .filter(|repository| repository.name == selector || repository.directory == selector)
        .collect::<Vec<_>>();
    let repository = match matches.as_slice() {
        [repository] => *repository,
        [] => {
            return Err(classified(
                ErrorCode::UnknownRepository,
                format!("unknown repository selector: {selector}"),
            ));
        }
        _ => {
            return Err(classified(
                ErrorCode::AmbiguousRepository,
                format!("ambiguous repository selector: {selector}"),
            ));
        }
    };
    let path = workspace::repository_path(&root, &repository.directory)?;
    let runtime = git::repository_runtime_info(&path);
    let remotes = repository
        .remotes
        .iter()
        .map(|remote| InfoRemote {
            name: remote.name.clone(),
            primary: remote.name == repository.primary_remote,
            fetch_url: git::display_remote_url(&remote.fetch_url),
            push_url: remote.push_url.as_deref().map(git::display_remote_url),
        })
        .collect::<Vec<_>>();
    let output = RepositoryInfoOutput {
        name: repository.name.clone(),
        directory: repository.directory.clone(),
        path: path.to_string_lossy().into_owned(),
        state: runtime.state.label(),
        current_branch: runtime.current_branch.clone(),
        default_branch: repository.default_branch.clone(),
        head: runtime.head.clone(),
        primary_remote: repository.primary_remote.clone(),
        local_branches: runtime.local_branches,
        remote_branches: runtime.remote_branches,
        created_at: repository.created_at.clone(),
        synced_at: repository.synced_at.clone(),
        remotes,
    };
    let exit_code = i32::from(runtime.state != RepositoryRuntimeState::Available);
    if automation.is_machine() {
        automation::emit_data(automation, "info", Some(&root), exit_code, &output)?;
    } else if arguments.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print_repository_info(&output, current_feature_branch.as_deref());
    }
    Ok(exit_code)
}

fn print_workspace_info(output: &WorkspaceInfoOutput) {
    let mut rows = vec![
        vec!["ROOT".to_owned(), output.root.clone()],
        vec!["MANIFEST".to_owned(), output.manifest.clone()],
        vec!["VERSION".to_owned(), output.version.to_string()],
        vec!["CREATED".to_owned(), output.created_at.clone()],
        vec!["UPDATED".to_owned(), output.updated_at.clone()],
    ];
    if let Some(branch) = &output.current_feature_branch {
        rows.push(vec!["FEATURE BRANCH".to_owned(), color::magenta(branch)]);
    }
    rows.extend([
        vec![
            "REPOSITORIES".to_owned(),
            output.repositories.total.to_string(),
        ],
        vec![
            "MATERIALIZED".to_owned(),
            color::green(output.repositories.materialized),
        ],
        vec![
            "MISSING".to_owned(),
            color::red(output.repositories.missing),
        ],
        vec![
            "NOT-GIT".to_owned(),
            color::red(output.repositories.not_git),
        ],
        vec!["BARE".to_owned(), color::red(output.repositories.bare)],
        vec!["ERROR".to_owned(), color::red(output.repositories.error)],
        vec!["JOBS".to_owned(), output.jobs.to_string()],
    ]);
    print!("{}", table::render(&["FIELD", "VALUE"], &rows));
}

fn print_repository_info(output: &RepositoryInfoOutput, feature_branch: Option<&str>) {
    let branches = match (output.local_branches, output.remote_branches) {
        (Some(local), Some(remote)) => format!("{local} local, {remote} remote"),
        _ => "-".to_owned(),
    };
    let rows = vec![
        vec!["NAME".to_owned(), output.name.clone()],
        vec!["DIRECTORY".to_owned(), output.directory.clone()],
        vec!["PATH".to_owned(), output.path.clone()],
        vec!["STATE".to_owned(), color_runtime_state(output.state)],
        vec![
            "CURRENT BRANCH".to_owned(),
            output.current_branch.as_ref().map_or_else(
                || "-".to_owned(),
                |branch| color::branch(branch, &output.default_branch, feature_branch),
            ),
        ],
        vec![
            "DEFAULT BRANCH".to_owned(),
            color::blue(&output.default_branch),
        ],
        vec![
            "HEAD".to_owned(),
            output.head.clone().unwrap_or_else(|| "-".to_owned()),
        ],
        vec!["PRIMARY REMOTE".to_owned(), output.primary_remote.clone()],
        vec!["BRANCHES".to_owned(), branches],
        vec!["CREATED".to_owned(), output.created_at.clone()],
        vec![
            "LAST SYNC".to_owned(),
            output.synced_at.clone().unwrap_or_else(|| "-".to_owned()),
        ],
    ];
    print!("{}", table::render(&["FIELD", "VALUE"], &rows));
    let remote_rows = output
        .remotes
        .iter()
        .map(|remote| {
            vec![
                remote.name.clone(),
                if remote.primary { "yes" } else { "no" }.to_owned(),
                remote.fetch_url.clone(),
            ]
        })
        .collect::<Vec<_>>();
    if !remote_rows.is_empty() {
        println!();
        print!(
            "{}",
            table::render(&["REMOTE", "PRIMARY", "FETCH URL"], &remote_rows)
        );
    }
}

fn color_runtime_state(state: &str) -> String {
    match state {
        "available" => color::green(state),
        "missing" | "not-git" | "bare" | "error" => color::red(state),
        _ => color::yellow(state),
    }
}
