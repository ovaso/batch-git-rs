//! Side-effect-free planning for mutating commands.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde_json::json;

use super::{default_clone_directory, merge_settings, relative_string, select_exec_repositories};
use crate::automation::{self, AutomationOptions};
use crate::cli::{CloneArgs, Command, MergeArgs, ScanArgs};
use crate::error::{ErrorCode, classified};
use crate::git;
use crate::model::{RepositoryRecord, WORKSPACE_FILE, Workspace, validate_directory};
use crate::{settings, table, workspace};

/// Produce a reviewable, no-side-effect description of a mutating built-in command.
///
/// A plan deliberately describes only local, observable preconditions. Remote Git state can
/// change between planning and apply, so callers must not treat it as a distributed transaction.
pub(super) fn plan_command(
    command: &Command,
    jobs: usize,
    automation: &AutomationOptions,
) -> Result<i32> {
    #[cfg(feature = "schedule")]
    if matches!(command, Command::Schedule(_)) {
        return Err(classified(
            ErrorCode::InvalidArguments,
            "use schedule plan <name>, schedule doctor, or a command-specific --dry-run for schedule operations",
        ));
    }
    if !command.supports_global_plan() {
        return Err(classified(
            ErrorCode::InvalidArguments,
            "--plan is only valid for an operation with side effects",
        ));
    }
    if feature_branch_is_unset(command)? {
        return render_feature_branch_unset_plan(command, automation);
    }

    let root = match command {
        Command::Clone(_) => workspace::find_root_optional()?.unwrap_or(workspace::current_root()?),
        // `scan` and `restore` deliberately operate on the invoking directory, rather than a
        // manifest found in a parent. Keep plans on the identical root so their revision can be
        // used by the subsequent operation.
        Command::Scan(_) | Command::Restore => workspace::current_root()?,
        _ => workspace::find_root()?,
    };
    let manifest_snapshot = root
        .join(WORKSPACE_FILE)
        .is_file()
        .then(|| workspace::read_with_revision(&root))
        .transpose()?;
    let manifest = manifest_snapshot
        .as_ref()
        .map(|(manifest, _revision)| manifest);
    let revision = manifest_snapshot
        .as_ref()
        .map(|(_manifest, revision)| revision.as_str());
    if matches!(command, Command::Restore) && manifest.is_none() {
        bail!("restore requires {} in {}", WORKSPACE_FILE, root.display());
    }

    let (operation, selection, side_effects, risk) = match command {
        Command::Add(arguments) => {
            let manifest = manifest
                .as_ref()
                .expect("add requires a workspace manifest");
            (
                "add",
                plan_selection(&crate::selector::select(
                    manifest,
                    &arguments.selectors,
                    &arguments.matches,
                    arguments.all,
                )?),
                vec!["git_indexes"],
                "index",
            )
        }
        Command::Clone(arguments) => (
            "clone",
            plan_clone_selection(arguments, &root, manifest)?,
            vec!["repository", "workspace_manifest"],
            "network",
        ),
        Command::Scan(arguments) => (
            "scan",
            plan_scan_selection(arguments, &root, manifest)?,
            vec!["workspace_manifest"],
            "local_write",
        ),
        Command::Commit(arguments) => {
            let manifest = manifest
                .as_ref()
                .expect("commit requires a workspace manifest");
            (
                "commit",
                plan_selection(&crate::selector::select(
                    manifest,
                    &arguments.selection.selectors,
                    &arguments.selection.matches,
                    arguments.selection.all,
                )?),
                vec!["git_objects", "local_refs", "git_indexes", "hooks"],
                "local_history",
            )
        }
        Command::Restore => {
            let manifest = manifest
                .as_ref()
                .expect("restore requires a workspace manifest");
            (
                "restore",
                plan_selection(&manifest.repositories),
                vec!["repositories", "workspace_manifest"],
                "network",
            )
        }
        Command::Fetch => {
            let manifest = manifest
                .as_ref()
                .expect("fetch requires a workspace manifest");
            (
                "fetch",
                plan_selection(&manifest.repositories),
                vec!["local_refs", "workspace_manifest"],
                "network",
            )
        }
        Command::Sync(arguments) => {
            let manifest = manifest
                .as_ref()
                .expect("sync requires a workspace manifest");
            (
                "sync",
                plan_selection(&crate::selector::select(
                    manifest,
                    &arguments.selectors,
                    &arguments.matches,
                    arguments.all,
                )?),
                vec!["repositories", "local_refs", "workspace_manifest"],
                "network",
            )
        }
        Command::Pull(arguments) => {
            let manifest = manifest
                .as_ref()
                .expect("pull requires a workspace manifest");
            (
                "pull",
                plan_selection(&crate::selector::select(
                    manifest,
                    &arguments.selectors,
                    &arguments.matches,
                    arguments.all,
                )?),
                vec!["working_trees", "local_refs", "workspace_manifest"],
                "working_tree",
            )
        }
        Command::Push(arguments) => {
            let manifest = manifest
                .as_ref()
                .expect("push requires a workspace manifest");
            (
                "push",
                plan_selection(&crate::selector::select(
                    manifest,
                    &arguments.selection.selectors,
                    &arguments.selection.matches,
                    arguments.selection.all,
                )?),
                vec!["remote_refs"],
                "remote_write",
            )
        }
        Command::Checkout(_) | Command::Cd | Command::Cf => {
            let manifest = manifest
                .as_ref()
                .expect("checkout requires a workspace manifest");
            (
                "checkout",
                plan_selection(&manifest.repositories),
                vec!["working_trees", "local_refs"],
                "working_tree",
            )
        }
        Command::Merge(arguments) => {
            let manifest = manifest
                .as_ref()
                .expect("merge requires a workspace manifest");
            let merge_settings = merge_settings(arguments)?;
            let mut side_effects = vec!["working_trees", "local_refs", "workspace_manifest"];
            if merge_settings.refresh_source || merge_settings.update_current {
                side_effects.push("network");
            }
            (
                "merge",
                plan_merge_selection(
                    &manifest.repositories,
                    arguments,
                    merge_settings.refresh_source,
                )?,
                side_effects,
                "working_tree",
            )
        }
        Command::Exec(arguments) => {
            let manifest = manifest
                .as_ref()
                .expect("exec requires a workspace manifest");
            (
                "exec",
                plan_selection(&select_exec_repositories(manifest, arguments)?),
                vec!["unclassified_git_command"],
                "unclassified",
            )
        }
        Command::Forget(arguments) => {
            let manifest = manifest
                .as_ref()
                .expect("forget requires a workspace manifest");
            (
                "forget",
                plan_selection(&crate::selector::select(
                    manifest,
                    &arguments.selectors,
                    &[],
                    false,
                )?),
                vec!["workspace_manifest"],
                "metadata",
            )
        }
        Command::Unstage(arguments) => {
            let manifest = manifest
                .as_ref()
                .expect("unstage requires a workspace manifest");
            (
                "unstage",
                plan_selection(&crate::selector::select(
                    manifest,
                    &arguments.selectors,
                    &arguments.matches,
                    arguments.all,
                )?),
                vec!["git_indexes"],
                "index",
            )
        }
        Command::Branch(_)
        | Command::Capabilities
        | Command::Env(_)
        | Command::Find(_)
        | Command::Info(_)
        | Command::List(_)
        | Command::Schema(_)
        | Command::Status(_) => unreachable!("non-mutating commands were rejected above"),
        #[cfg(feature = "schedule")]
        Command::Schedule(_) => unreachable!("schedule planning was rejected above"),
    };

    let parameters = match command {
        Command::Add(_) => Some(json!({
            "scope": "all_working_tree_changes",
            "includes": ["additions", "modifications", "deletions"],
            "force_ignored": false,
        })),
        Command::Commit(arguments) => Some(json!({
            "message": arguments.message.as_str(),
            "stages_content": false,
        })),
        Command::Unstage(_) => Some(json!({
            "scope": "all_staged_changes",
            "preserves_working_tree": true,
            "moves_head": false,
        })),
        Command::Merge(arguments) => Some(json!({
            "update_current": merge_settings(arguments)?.update_current,
            "refresh_source": merge_settings(arguments)?.refresh_source,
        })),
        _ => None,
    };
    render_plan(
        PlanSpec {
            operation,
            repositories: selection,
            side_effects,
            risk,
            parameters,
        },
        &root,
        jobs,
        revision,
        automation,
    )
}

/// `cf`, `checkout --feature`, and `merge --feature` are documented no-ops when their input
/// variable is absent. Preserve that behavior for planning without inventing repository records.
fn feature_branch_is_unset(command: &Command) -> Result<bool> {
    let requires_feature_branch = matches!(command, Command::Cf)
        || matches!(command, Command::Checkout(arguments) if arguments.feature)
        || matches!(command, Command::Merge(arguments) if arguments.feature);
    Ok(requires_feature_branch && settings::current_feature_branch()?.is_none())
}

fn render_feature_branch_unset_plan(
    command: &Command,
    automation: &AutomationOptions,
) -> Result<i32> {
    let operation = if matches!(command, Command::Merge(_)) {
        "merge"
    } else {
        "checkout"
    };
    let data = json!({
        "mode": "plan",
        "status": "skipped",
        "reason_code": "feature_branch_unset",
        "detail": "CURRENT_FEATURE_BRANCH is not set; no repository operation is planned.",
        "side_effects": [],
    });
    if automation.is_machine() {
        automation::emit_data(automation, operation, None, 0, &data)?;
    } else {
        println!("CURRENT_FEATURE_BRANCH is not set; nothing to {operation}");
    }
    Ok(0)
}

/// Plan Git passthrough without attempting to classify untrusted Git arguments.
pub(super) fn plan_passthrough(
    _args: &[OsString],
    root: &Path,
    jobs: usize,
    automation: &AutomationOptions,
) -> Result<i32> {
    let (manifest, revision) = workspace::read_with_revision(root)?;
    render_plan(
        PlanSpec {
            operation: "passthrough",
            repositories: plan_selection(&manifest.repositories),
            side_effects: vec!["unclassified_git_command"],
            risk: "unclassified",
            parameters: None,
        },
        root,
        jobs,
        Some(&revision),
        automation,
    )
}

/// Resolve the exact target directory a clone would create without contacting the remote.
fn plan_clone_selection(
    arguments: &CloneArgs,
    root: &Path,
    manifest: Option<&Workspace>,
) -> Result<Vec<serde_json::Value>> {
    if let Some(depth) = arguments.depth
        && depth == 0
    {
        return Err(classified(
            ErrorCode::InvalidArguments,
            "--depth must be at least 1",
        ));
    }
    let directory = arguments
        .directory
        .clone()
        .unwrap_or_else(|| PathBuf::from(default_clone_directory(&arguments.repository)));
    let directory_string = relative_string(&directory)?;
    validate_directory(&directory_string)?;
    if manifest.is_some_and(|manifest| {
        manifest
            .repositories
            .iter()
            .any(|repository| repository.directory == directory_string)
    }) {
        bail!("repository directory is already registered: {directory_string}");
    }
    let target = workspace::repository_path(root, &directory_string)?;
    if target.exists() {
        bail!("clone destination already exists: {}", target.display());
    }
    Ok(vec![json!({
        "directory": directory_string,
        "source": git::display_remote_url(&arguments.repository),
    })])
}

/// Discover the local directories a scan would inspect without writing a manifest or using Git
/// network operations. Each directory is a candidate because inspection can still fail later on
/// a corrupted repository, which is reported per repository by the actual scan.
fn plan_scan_selection(
    arguments: &ScanArgs,
    root: &Path,
    manifest: Option<&Workspace>,
) -> Result<Vec<serde_json::Value>> {
    let known_directories = manifest
        .map(|manifest| {
            manifest
                .repositories
                .iter()
                .map(|repository| repository.directory.as_str())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let depth = settings::scan_depth(arguments.depth)?;
    Ok(git::discover(root, depth)?
        .into_iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(root).ok()?;
            let directory = relative_string(relative).ok()?;
            (!known_directories.contains(directory.as_str())).then_some(json!({
                "directory": directory,
                "status": "candidate",
            }))
        })
        .collect())
}

/// Convert manifest records into the stable core selection shape used by plans.
fn plan_selection(records: &[RepositoryRecord]) -> Vec<serde_json::Value> {
    records
        .iter()
        .map(|record| {
            json!({
                "name": record.name,
                "directory": record.directory,
                "default_branch": record.default_branch,
            })
        })
        .collect()
}

/// Resolve the per-repository merge source without touching Git state or the network.
fn plan_merge_selection(
    records: &[RepositoryRecord],
    arguments: &MergeArgs,
    refresh_source: bool,
) -> Result<Vec<serde_json::Value>> {
    let feature_branch = if arguments.feature {
        settings::current_feature_branch()?
    } else {
        None
    };
    let shared_branch = arguments.branch.as_deref().or(feature_branch.as_deref());
    let selected_remote = if arguments.default {
        None
    } else {
        settings::checkout_remote(arguments.remote.clone())
    };
    let source_mode = if arguments.default {
        "workspace_default"
    } else if arguments.feature {
        "feature_environment"
    } else {
        "explicit"
    };

    Ok(records
        .iter()
        .map(|record| {
            let source_branch = if arguments.default {
                record.default_branch.as_str()
            } else {
                shared_branch.expect("clap requires a branch, --feature, or --default")
            };
            let source_remote = if arguments.default {
                Some(record.primary_remote.as_str())
            } else {
                selected_remote.as_deref()
            };
            json!({
                "name": record.name,
                "directory": record.directory,
                "default_branch": record.default_branch,
                "source_branch": source_branch,
                "source_mode": source_mode,
                "remote_fallback": source_remote,
                "source_refresh_remote": refresh_source.then(|| {
                    source_remote.unwrap_or(record.primary_remote.as_str())
                }),
            })
        })
        .collect())
}

/// The operation-specific portion of a no-side-effect plan.
struct PlanSpec {
    operation: &'static str,
    repositories: Vec<serde_json::Value>,
    side_effects: Vec<&'static str>,
    risk: &'static str,
    parameters: Option<serde_json::Value>,
}

/// Serialize or render the common plan payload. No caller of this function performs a write.
fn render_plan(
    plan: PlanSpec,
    root: &Path,
    jobs: usize,
    revision: Option<&str>,
    automation: &AutomationOptions,
) -> Result<i32> {
    let apply_available = revision.is_some();
    let apply_note = if apply_available {
        "Apply rechecks batchspace.toml only; it does not reserve remote or Git state."
    } else {
        "No batchspace.toml revision exists yet; request explicit approval, then run the operation without --apply."
    };
    let mut data = json!({
        "mode": "plan",
        "atomicity": "per_repository",
        "jobs": jobs,
        "workspace_revision": revision,
        "selection": {"repositories": plan.repositories},
        "side_effects": plan.side_effects,
        "risk": plan.risk,
        "apply": {
            "requires_workspace_revision": apply_available,
            "note": apply_note
        }
    });
    if let Some(parameters) = &plan.parameters {
        data["parameters"] = parameters.clone();
    }
    if automation.is_machine() {
        automation::emit_data_with_workspace_revision(
            automation,
            plan.operation,
            Some(root),
            revision,
            0,
            &data,
        )?;
    } else {
        let show_merge_source = plan
            .repositories
            .iter()
            .any(|repository| repository.get("source_branch").is_some());
        let rows = plan
            .repositories
            .iter()
            .map(|repository| {
                let mut row = vec![
                    repository
                        .get("name")
                        .or_else(|| repository.get("source"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("-")
                        .to_owned(),
                    repository
                        .get("directory")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("-")
                        .to_owned(),
                ];
                if show_merge_source {
                    row.push(
                        repository
                            .get("source_branch")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("-")
                            .to_owned(),
                    );
                }
                row
            })
            .collect::<Vec<_>>();
        if rows.is_empty() {
            println!(
                "plan: {}; no registered repositories are resolved yet",
                plan.operation
            );
        } else {
            let headers = if show_merge_source {
                vec!["REPOSITORY", "DIRECTORY", "SOURCE"]
            } else {
                vec!["REPOSITORY", "DIRECTORY"]
            };
            print!("{}", table::render(&headers, &rows));
        }
        println!(
            "plan: {}; risk: {}; side effects: {}",
            plan.operation,
            plan.risk,
            plan.side_effects.join(", ")
        );
        if let Some(parameters) = &plan.parameters {
            println!("parameters: {}", serde_json::to_string(parameters)?);
        }
        match revision {
            Some(revision) => println!(
                "apply: batch-git --apply --expect-workspace-revision {revision} {} …",
                plan.operation
            ),
            None => {
                println!(
                    "apply: request approval, then run this first workspace-creation operation directly"
                )
            }
        }
    }
    Ok(0)
}
