//! Built-in command orchestration.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use serde::Serialize;
use serde_json::json;

use crate::automation::{self, AutomationOptions};
use crate::cli::{
    CheckoutArgs, Cli, CloneArgs, Command, CommitArgs, ExecArgs, FindArgs, ForgetArgs, InfoArgs,
    ListArgs, MachineReadableArgs, MergeArgs, PushArgs, RuntimeOptions, ScanArgs, SchemaArgs,
    SchemaDocument, SyncArgs,
};
use crate::color;
use crate::git::{
    self, BranchKind, CheckoutTarget, CloneOptions, GitExecutionOptions, GitOutput,
    RepositoryRuntimeState, UpstreamSummary,
};
use crate::model::{RepositoryRecord, WORKSPACE_FILE, Workspace, now, validate_directory};
use crate::parallel::{map_ordered, map_ordered_with_completion};
use crate::report::{
    JsonlProgress, RepositoryResult, print_checkout_summary, print_push_summary, print_results,
    print_selected_results,
};
use crate::settings;
use crate::table;
use crate::workspace::{self, WorkspaceLock};

/// 解析公共运行配置，并把顶层子命令分派到对应工作流。
pub fn dispatch(cli: Cli) -> Result<i32> {
    let runtime = cli.runtime_options()?;
    let jobs = settings::jobs(runtime.jobs)?;
    let automation = runtime.automation();
    if let Command::Commit(arguments) = &cli.command {
        validate_commit_message(&arguments.message)?;
    }
    if automation.plan {
        return plan_command(&cli.command, jobs, &automation);
    }
    if automation.apply && !cli.command.is_mutating() {
        bail!("--apply is only valid for an operation with side effects");
    }
    match cli.command {
        Command::Add(arguments) => add(arguments, jobs, runtime.verbose, &automation),
        Command::Clone(arguments) => clone_repository(arguments, &automation),
        Command::Commit(arguments) => commit(arguments, jobs, runtime.verbose, &automation),
        Command::Scan(arguments) => scan(arguments, jobs, &automation),
        Command::Restore => restore(jobs, runtime.verbose, &automation),
        Command::Fetch => fetch(jobs, runtime.verbose, &automation),
        Command::Sync(arguments) => sync(arguments, jobs, runtime.verbose, &automation),
        Command::Schedule(arguments) => {
            crate::schedule::dispatch(arguments, jobs, runtime.verbose, &automation)
        }
        Command::Checkout(arguments) => checkout(arguments, jobs, runtime.verbose, &automation),
        Command::Cd => checkout(
            checkout_alias_arguments(true, false),
            jobs,
            runtime.verbose,
            &automation,
        ),
        Command::Cf => checkout(
            checkout_alias_arguments(false, true),
            jobs,
            runtime.verbose,
            &automation,
        ),
        Command::Merge(arguments) => merge(arguments, jobs, runtime.verbose, &automation),
        Command::Pull(arguments) => pull(arguments, jobs, runtime.verbose, &automation),
        Command::Push(arguments) => push(arguments, jobs, runtime.verbose, &automation),
        Command::Exec(arguments) => exec(arguments, jobs, runtime.verbose, &automation),
        Command::Status(arguments) => status(arguments, jobs, &automation),
        Command::Find(arguments) => find(arguments, jobs, &automation),
        Command::Info(arguments) => info(arguments, jobs, &automation),
        Command::Branch(arguments) => branch(arguments, jobs, &automation),
        Command::Capabilities => capabilities(&automation),
        Command::Schema(arguments) => schema(arguments, &automation),
        Command::List(arguments) => list(arguments, jobs, &automation),
        Command::Forget(arguments) => forget(arguments, &automation),
        Command::Unstage(arguments) => unstage(arguments, jobs, runtime.verbose, &automation),
    }
}

/// Produce a reviewable, no-side-effect description of a mutating built-in command.
///
/// A plan deliberately describes only local, observable preconditions. Remote Git state can
/// change between planning and apply, so callers must not treat it as a distributed transaction.
fn plan_command(command: &Command, jobs: usize, automation: &AutomationOptions) -> Result<i32> {
    if !command.is_mutating() {
        bail!("--plan is only valid for an operation with side effects");
    }
    if matches!(command, Command::Schedule(_)) {
        bail!(
            "use schedule plan <name>, schedule doctor, or a command-specific --dry-run for schedule operations"
        );
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
        | Command::Find(_)
        | Command::Info(_)
        | Command::List(_)
        | Command::Schema(_)
        | Command::Status(_)
        | Command::Schedule(_) => unreachable!("non-mutating commands were rejected above"),
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
fn plan_passthrough(
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
        bail!("--depth must be at least 1");
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
    let target = root.join(&directory);
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

fn merge_update_current_setting(arguments: &MergeArgs) -> Option<bool> {
    if arguments.update_current {
        Some(true)
    } else if arguments.no_update_current {
        Some(false)
    } else {
        None
    }
}

fn merge_refresh_source_setting(arguments: &MergeArgs) -> Option<bool> {
    if arguments.refresh_source {
        Some(true)
    } else if arguments.no_refresh_source {
        Some(false)
    } else {
        None
    }
}

struct MergeSettings {
    update_current: bool,
    refresh_source: bool,
}

/// Resolve source-specific defaults first, then let explicit CLI flags override them.
fn merge_settings(arguments: &MergeArgs) -> Result<MergeSettings> {
    let update_current = match merge_update_current_setting(arguments) {
        Some(value) => value,
        None if arguments.feature => settings::merge_feature_update_current()?,
        None => false,
    };
    let refresh_source = match merge_refresh_source_setting(arguments) {
        Some(value) => value,
        None if arguments.default => settings::merge_default_refresh_source()?,
        None => false,
    };
    Ok(MergeSettings {
        update_current,
        refresh_source,
    })
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
        "Apply rechecks workspace.toml only; it does not reserve remote or Git state."
    } else {
        "No workspace.toml revision exists yet; request explicit approval, then run the operation without --apply."
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

/// Enforce the manifest precondition carried from a preceding plan before a write begins.
fn verify_apply_revision(root: &Path, automation: &AutomationOptions) -> Result<()> {
    if let Some(expected) = automation.expected_workspace_revision.as_deref() {
        workspace::verify_revision(root, expected)?;
    }
    Ok(())
}

/// Return the current binary's protocol surface rather than requiring agents to guess it.
fn capabilities(automation: &AutomationOptions) -> Result<i32> {
    let data = json!({
        "binary_version": env!("CARGO_PKG_VERSION"),
        "automation_protocol": automation::API_VERSION,
        "output_formats": ["text", "json", "jsonl"],
        "schemas": ["operation-result", "workspace"],
        "commands": [
            "add", "branch", "capabilities", "checkout", "clone", "commit", "exec",
            "fetch", "find", "forget", "info", "list", "merge", "pull", "push",
            "restore", "scan", "schedule", "schema", "status", "sync", "unstage",
            "passthrough"
        ],
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

/// Print a JSON Schema document. Text mode prints the schema itself for shell-friendly use;
/// `--output json` wraps it in the normal v1 receipt.
fn schema(arguments: SchemaArgs, automation: &AutomationOptions) -> Result<i32> {
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
        "$id": "https://github.com/livenv/batch-git/schemas/operation-result-v1.json",
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
        "$id": "https://github.com/livenv/batch-git/schemas/workspace-v1.json",
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

/// Resolve the `exec` subset once, preserving manifest order and its precise selector errors.
fn select_exec_repositories(
    manifest: &Workspace,
    arguments: &ExecArgs,
) -> Result<Vec<RepositoryRecord>> {
    crate::selector::select(manifest, &arguments.selectors, &arguments.matches, false)
}

/// Derive one child-process policy for an invocation. Machine output is necessarily
/// non-interactive: inheriting a terminal child would violate the JSON stdout contract.
fn git_execution_options(automation: &AutomationOptions, allow_stdin: bool) -> GitExecutionOptions {
    let non_interactive = automation.non_interactive || automation.is_machine();
    GitExecutionOptions {
        allow_stdin: allow_stdin && !non_interactive,
        non_interactive,
        timeout: automation.timeout,
    }
}

/// Execute repository work with JSONL lifecycle events and a stable result vector.
///
/// `JsonlProgress` emits `started` before workers begin, then preserves the protocol's manifest
/// ordering by buffering out-of-order worker completions until their predecessors are available.
fn map_repository_results<T, F>(
    items: &[T],
    jobs: usize,
    automation: &AutomationOptions,
    command: &str,
    root: &Path,
    operation: F,
) -> Result<Vec<RepositoryResult>>
where
    T: Sync,
    F: Fn(&T) -> RepositoryResult + Sync + Send,
{
    let progress = JsonlProgress::new(automation, command, root, items.len())?;
    map_ordered_with_completion(items, jobs, operation, |index, result| {
        if let Some(progress) = &progress {
            progress.repository_finished(index, result);
        }
    })
}

/// 将 `cd`、`cf` 两个便捷别名转换为标准 checkout 参数。
fn checkout_alias_arguments(default: bool, feature: bool) -> CheckoutArgs {
    CheckoutArgs {
        create: false,
        branch: None,
        default,
        feature,
        from: None,
        remote: None,
    }
}

/// 在所有已物化仓库中原样执行 `--` 后的 Git 参数。
pub fn passthrough(args: Vec<OsString>, options: RuntimeOptions) -> Result<i32> {
    let jobs = settings::jobs(options.jobs)?;
    let automation = options.automation();
    automation.validate()?;
    let root = workspace::find_root()?;
    if automation.plan {
        return plan_passthrough(&args, &root, jobs, &automation);
    }
    // 即使看似只读的 Git 子命令也可能修改仓库，因此透传统一获取工作区锁。
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, &automation)?;
    let workspace = workspace::read(&root)?;
    let verbose = options.verbose || settings::passthrough_verbose()?;
    let results = map_repository_results(
        &workspace.repositories,
        jobs,
        &automation,
        "passthrough",
        &root,
        |repository| {
            let path = root.join(&repository.directory);
            if !git::is_repository(&path) {
                return RepositoryResult::skipped(repository, "repository is not materialized");
            }
            match git::run_os_with_options(
                &path,
                &args,
                false,
                git_execution_options(&automation, jobs == 1),
            ) {
                Ok(output) => passthrough_result(repository, &args, output),
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            }
        },
    )?;
    print_results(&results, verbose, &automation, "passthrough", &root)
}

/// 在用户精确选择的仓库子集中执行原生 Git 命令。
fn exec(
    arguments: ExecArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let workspace = workspace::read(&root)?;
    let mut selected = HashSet::new();

    for selector in &arguments.selectors {
        let matches = workspace
            .repositories
            .iter()
            .enumerate()
            .filter_map(|(index, repository)| {
                (repository.name == *selector || repository.directory == *selector).then_some(index)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [index] => {
                selected.insert(*index);
            }
            [] => bail!("unknown repository selector: {selector}"),
            _ => bail!("ambiguous repository selector: {selector}"),
        }
    }

    for pattern in &arguments.matches {
        let matches = workspace
            .repositories
            .iter()
            .enumerate()
            .filter_map(|(index, repository)| {
                wildcard_matches(pattern, &repository.name).then_some(index)
            })
            .collect::<Vec<_>>();
        if matches.is_empty() {
            bail!("repository pattern matched nothing: {pattern}");
        }
        selected.extend(matches);
    }

    let repositories = workspace
        .repositories
        .iter()
        .enumerate()
        .filter_map(|(index, repository)| selected.contains(&index).then_some(repository.clone()))
        .collect::<Vec<_>>();
    let results = map_repository_results(
        &repositories,
        jobs,
        automation,
        "exec",
        &root,
        |repository| {
            let path = root.join(&repository.directory);
            if !git::is_repository(&path) {
                return RepositoryResult::failed(repository, "repository is not materialized");
            }
            match git::run_os_with_options(
                &path,
                &arguments.git_args,
                false,
                git_execution_options(automation, jobs == 1),
            ) {
                Ok(output) => passthrough_result(repository, &arguments.git_args, output),
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            }
        },
    )?;
    print_selected_results(&results, verbose, automation, "exec", &root)
}

/// 把 Git 子进程结果转换为统一的仓库级结果。
fn passthrough_result(
    repository: &RepositoryRecord,
    args: &[OsString],
    output: GitOutput,
) -> RepositoryResult {
    if is_nothing_to_commit(args, &output) {
        RepositoryResult::skipped_from_git(repository, "nothing to commit", output)
    } else {
        RepositoryResult::from_git(repository, output, "Git command completed", false)
    }
}

/// 识别 `git commit` 的“没有内容可提交”，将其视为跳过而非批量失败。
fn is_nothing_to_commit(args: &[OsString], output: &GitOutput) -> bool {
    if output.success || output.code != Some(1) || args.first().is_none_or(|arg| arg != "commit") {
        return false;
    }
    let message = format!("{}\n{}", output.stdout, output.stderr).to_ascii_lowercase();
    [
        "nothing to commit",
        "no changes added to commit",
        "nothing added to commit",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

/// 克隆单个仓库；只有 clone 和检查全部成功后才写入清单。
fn clone_repository(arguments: CloneArgs, automation: &AutomationOptions) -> Result<i32> {
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
    // Keep a failed clone directory for inspection instead of recursively deleting a path that a
    // concurrent process could have populated after our reservation. The empty reservation also
    // makes Git's destination check atomic from batch-git's perspective.
    // A clone has exactly one repository-sized unit of work. Start its JSONL lifecycle after all
    // local validation and reservation complete, but before Git receives control.
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
                // A failed initial clone has no canonical manifest name yet. Its requested
                // directory is nevertheless a stable identifier for JSONL consumers.
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

/// 扫描已有仓库并增量补充清单，不删除既有登记。
fn scan(arguments: ScanArgs, jobs: usize, automation: &AutomationOptions) -> Result<i32> {
    let root = workspace::current_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let mut manifest = workspace::read_or_new(&root)?;
    let depth = settings::scan_depth(arguments.depth)?;
    let paths = git::discover(&root, depth)?;
    let known_directories: HashSet<String> = manifest
        .repositories
        .iter()
        .map(|repository| repository.directory.clone())
        .collect();
    let candidates: Vec<(PathBuf, String)> = paths
        .into_iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(&root).ok()?;
            let directory = relative_string(relative).ok()?;
            (!known_directories.contains(&directory)).then_some((path, directory))
        })
        .collect();

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
                    // Inspection did not produce a canonical name. Use the workspace-relative
                    // directory as the stable item identifier rather than omitting the common
                    // batch-result field altogether.
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

/// 根据清单克隆所有缺失仓库，并同步声明的远端配置。
fn restore(jobs: usize, verbose: bool, automation: &AutomationOptions) -> Result<i32> {
    let root = workspace::current_root()?;
    if !root.join(WORKSPACE_FILE).is_file() {
        bail!("restore requires {} in {}", WORKSPACE_FILE, root.display());
    }
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let mut manifest = workspace::read(&root)?;
    let records = manifest.repositories.clone();
    let progress = OperationProgress::new(&records, automation.is_machine());
    let results =
        map_repository_results(&records, jobs, automation, "restore", &root, |repository| {
            let bar = progress.bar(repository);
            restore_one(
                &root,
                repository,
                git_execution_options(automation, jobs == 1),
                bar.as_ref(),
            )
        })?;
    progress.finish();
    for (record, outcome) in manifest.repositories.iter_mut().zip(&results) {
        if outcome.was_synced() {
            record.synced_at = Some(now());
        }
    }
    if results.iter().any(RepositoryResult::was_synced) {
        workspace::write(&root, &mut manifest)?;
    }
    crate::report::print_operation_summary(&results, verbose, automation, "restore", &root)
}

/// 恢复或验证单个仓库，返回可聚合结果而不影响其他仓库。
fn restore_one(
    root: &Path,
    repository: &RepositoryRecord,
    execution: GitExecutionOptions,
    progress: Option<&ProgressBar>,
) -> RepositoryResult {
    set_operation_status(progress, "checking", 0);
    let target = root.join(&repository.directory);
    if target.exists() {
        if !git::is_repository(&target) {
            let result =
                RepositoryResult::failed(repository, "target exists but is not a Git repository");
            finish_operation(progress, &result);
            return result;
        }
        let result = match git::verify_declared_remotes(&target, repository).and_then(|()| {
            git::configure_declared_remotes(&target, repository, execution.allow_stdin)
        }) {
            Ok(()) => RepositoryResult::skipped(repository, "existing repository verified"),
            Err(error) => RepositoryResult::failed(repository, error.to_string()),
        };
        finish_operation(progress, &result);
        return result;
    }

    let Some(primary) = repository
        .remotes
        .iter()
        .find(|remote| remote.name == repository.primary_remote)
    else {
        let result = RepositoryResult::failed(repository, "primary remote is not declared");
        finish_operation(progress, &result);
        return result;
    };
    if let Err(error) = reserve_clone_destination(&target) {
        let result = RepositoryResult::failed(repository, error.to_string());
        finish_operation(progress, &result);
        return result;
    }
    set_operation_status(progress, "cloning", 0);
    let clone_progress = progress.cloned().map(|bar| {
        Arc::new(move |received: usize, total: usize| {
            let percentage = received.saturating_mul(100).checked_div(total).unwrap_or(0);
            bar.set_position(percentage as u64);
            bar.set_message(format!("receiving {received}/{total}"));
        }) as git::CloneProgress
    });
    match git::clone_repository_with_options(
        &primary.fetch_url,
        &target,
        CloneOptions {
            remote_name: &repository.primary_remote,
            branch: Some(&repository.default_branch),
            depth: None,
            single_branch: true,
            progress: clone_progress,
            allow_stdin: execution.allow_stdin,
        },
        execution,
    ) {
        Ok(_) => {}
        Err(error) => {
            // Do not recursively remove a failed destination. A concurrent process can populate
            // it after reservation; retaining it is safer and makes recovery an explicit choice.
            let result = RepositoryResult::failed(repository, error.to_string());
            finish_operation(progress, &result);
            return result;
        }
    }
    set_operation_status(progress, "configuring", 98);
    if let Err(error) = git::configure_declared_remotes(&target, repository, execution.allow_stdin)
    {
        let result = RepositoryResult::failed(repository, error.to_string());
        finish_operation(progress, &result);
        return result;
    }
    let result = RepositoryResult::success(repository, "restored default branch", true);
    finish_operation(progress, &result);
    result
}

/// clone/restore/fetch 共用的多仓库终端进度状态。
struct OperationProgress {
    multi: Option<MultiProgress>,
    bars: Vec<(String, ProgressBar)>,
}

impl OperationProgress {
    /// 仅在交互终端创建进度条；管道和 CI 中保持稳定表格输出。
    fn new(repositories: &[RepositoryRecord], machine: bool) -> Self {
        if machine || !io::stderr().is_terminal() {
            return Self {
                multi: None,
                bars: Vec::new(),
            };
        }
        let multi = MultiProgress::new();
        let name_width = repositories
            .iter()
            .map(|repository| repository.name.chars().count())
            .max()
            .unwrap_or(10)
            .max("REPOSITORY".len());
        let style = ProgressStyle::with_template(&format!(
            "{{prefix:<{name_width}}}  [{{bar:28.cyan/blue}}] {{pos:>3}}%  {{msg}}"
        ))
        .expect("valid operation progress template")
        .progress_chars("━━╸");
        let bars = repositories
            .iter()
            .map(|repository| {
                let bar = multi.add(ProgressBar::new(100));
                bar.set_style(style.clone());
                bar.set_prefix(repository.name.clone());
                bar.set_message("queued");
                (repository.name.clone(), bar)
            })
            .collect();
        Self {
            multi: Some(multi),
            bars,
        }
    }

    fn bar(&self, repository: &RepositoryRecord) -> Option<ProgressBar> {
        self.bars
            .iter()
            .find(|(name, _)| name == &repository.name)
            .map(|(_, bar)| bar.clone())
    }

    fn finish(self) {
        if let Some(multi) = self.multi {
            let _ = multi.clear();
        }
    }
}

/// 更新可选进度条的阶段文本和位置。
fn set_operation_status(progress: Option<&ProgressBar>, message: &'static str, position: u64) {
    if let Some(progress) = progress {
        progress.set_position(position);
        progress.set_message(message);
    }
}

/// 用仓库最终状态结束进度条。
fn finish_operation(progress: Option<&ProgressBar>, result: &RepositoryResult) {
    if let Some(progress) = progress {
        progress.set_position(100);
        progress.finish_with_message(result.progress_label());
    }
}

/// 对所有已物化仓库执行 fetch/prune，不改变工作树。
fn fetch(jobs: usize, verbose: bool, automation: &AutomationOptions) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let mut manifest = workspace::read(&root)?;
    let records = manifest.repositories.clone();
    let progress = OperationProgress::new(&records, automation.is_machine());
    let results =
        map_repository_results(&records, jobs, automation, "fetch", &root, |repository| {
            let bar = progress.bar(repository);
            set_operation_status(bar.as_ref(), "checking", 0);
            let path = root.join(&repository.directory);
            if !git::is_repository(&path) {
                let result = RepositoryResult::failed(
                    repository,
                    "repository is not materialized; run restore",
                );
                finish_operation(bar.as_ref(), &result);
                return result;
            }
            set_operation_status(bar.as_ref(), "configuring", 0);
            if let Err(error) = git::configure_declared_remotes(
                &path,
                repository,
                git_execution_options(automation, jobs == 1).allow_stdin,
            ) {
                let result = RepositoryResult::failed(repository, error.to_string());
                finish_operation(bar.as_ref(), &result);
                return result;
            }
            set_operation_status(bar.as_ref(), "fetching", 0);
            let result = match git::fetch_all_with_options(
                &path,
                git_execution_options(automation, jobs == 1),
            ) {
                Ok(output) => {
                    RepositoryResult::from_git(repository, output, "remote refs updated", true)
                }
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            };
            finish_operation(bar.as_ref(), &result);
            result
        })?;
    progress.finish();
    for (record, outcome) in manifest.repositories.iter_mut().zip(&results) {
        if outcome.was_synced() {
            record.synced_at = Some(now());
        }
    }
    if results.iter().any(RepositoryResult::was_synced) {
        workspace::write(&root, &mut manifest)?;
    }
    crate::report::print_operation_summary(&results, verbose, automation, "fetch", &root)
}

/// 解析用户选择后执行可供无人值守使用的 restore + fetch。
fn sync(
    arguments: SyncArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let mut manifest = workspace::read(&root)?;
    let records = crate::selector::select(
        &manifest,
        &arguments.selectors,
        &arguments.matches,
        arguments.all,
    )?;
    run_sync(&root, &mut manifest, &records, jobs, verbose, automation)
}

/// 执行已确定范围的同步；供命令行和 schedule 共用。
pub(crate) fn run_sync(
    root: &Path,
    manifest: &mut Workspace,
    records: &[RepositoryRecord],
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    run_sync_named(root, manifest, records, jobs, verbose, automation, "sync")
}

/// Execute sync with the externally visible operation name supplied by a parent workflow.
pub(crate) fn run_sync_named(
    root: &Path,
    manifest: &mut Workspace,
    records: &[RepositoryRecord],
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
    command: &str,
) -> Result<i32> {
    let progress = OperationProgress::new(records, automation.is_machine());
    let results = map_repository_results(records, jobs, automation, command, root, |repository| {
        let bar = progress.bar(repository);
        set_operation_status(bar.as_ref(), "checking", 0);
        let restore_result = restore_one(
            root,
            repository,
            git_execution_options(automation, jobs == 1),
            None,
        );
        if restore_result.is_failed() {
            finish_operation(bar.as_ref(), &restore_result);
            return restore_result;
        }
        let restored = restore_result.was_synced();
        set_operation_status(bar.as_ref(), "fetching", 0);
        let path = root.join(&repository.directory);
        let result = match git::fetch_all_with_options(
            &path,
            git_execution_options(automation, jobs == 1),
        ) {
            Ok(output) => RepositoryResult::from_git(
                repository,
                output,
                if restored {
                    "restored and remote refs updated"
                } else {
                    "remote refs updated"
                },
                true,
            ),
            Err(error) => RepositoryResult::failed(repository, error.to_string()),
        };
        finish_operation(bar.as_ref(), &result);
        result
    })?;
    progress.finish();

    let timestamp = now();
    let mut changed = false;
    for (selected, outcome) in records.iter().zip(&results) {
        if !outcome.was_synced() {
            continue;
        }
        if let Some(record) = manifest
            .repositories
            .iter_mut()
            .find(|record| record.name == selected.name)
        {
            record.synced_at = Some(timestamp.clone());
            changed = true;
        }
    }
    if changed {
        workspace::write(root, manifest)?;
    }
    crate::report::print_operation_summary(&results, verbose, automation, command, root)
}

/// Stage every tracked, untracked, and deleted path in the selected repositories.
fn add(
    arguments: SyncArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let execution = git_execution_options(automation, false);
    run_selected_local_operation(
        arguments,
        jobs,
        verbose,
        automation,
        "add",
        |repository, path| {
            let state = match git::staging_state(path) {
                Ok(state) => state,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            if state.has_conflicts {
                return RepositoryResult::failed(
                    repository,
                    "repository has unresolved conflicts; use exec with an explicit pathspec to stage resolutions",
                );
            }
            if !state.has_worktree_changes {
                return RepositoryResult::skipped(repository, "nothing to stage");
            }
            match git::run_with_options(path, ["add", "--all", "--", ":/"], true, execution) {
                Ok(output) => RepositoryResult::from_git(
                    repository,
                    output,
                    "all working-tree changes staged",
                    false,
                ),
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            }
        },
    )
}

/// Commit only content that was already present in each selected repository's index.
fn commit(
    arguments: CommitArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let CommitArgs { selection, message } = arguments;
    let execution = git_execution_options(automation, jobs == 1);
    run_selected_local_operation(
        selection,
        jobs,
        verbose,
        automation,
        "commit",
        move |repository, path| {
            let branch = match git::current_branch_summary(path) {
                Ok(branch) => branch,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            if branch.starts_with("(detached:") {
                return RepositoryResult::failed(
                    repository,
                    "current HEAD is detached; checkout a local branch before committing",
                );
            }
            let state = match git::staging_state(path) {
                Ok(state) => state,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            if state.has_conflicts {
                return RepositoryResult::failed(repository, "repository has unresolved conflicts");
            }
            match git::operation_in_progress(path) {
                Ok(true) => {
                    return RepositoryResult::failed(
                        repository,
                        "repository operation is in progress; use an explicit Git workflow to continue it",
                    );
                }
                Ok(false) => {}
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            }
            if !state.has_staged_changes {
                return RepositoryResult::skipped(repository, "nothing to commit");
            }
            match git::run_with_options(path, ["commit", "-m", message.as_str()], true, execution) {
                Ok(output) => RepositoryResult::from_git(
                    repository,
                    output,
                    "committed staged changes",
                    false,
                ),
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            }
        },
    )
}

fn validate_commit_message(message: &str) -> Result<()> {
    if message.trim().is_empty() {
        bail!("commit message cannot be empty");
    }
    Ok(())
}

/// Restore each selected index to HEAD without updating any working-tree file.
fn unstage(
    arguments: SyncArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let execution = git_execution_options(automation, false);
    run_selected_local_operation(
        arguments,
        jobs,
        verbose,
        automation,
        "unstage",
        |repository, path| {
            let state = match git::staging_state(path) {
                Ok(state) => state,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            if !state.has_staged_changes {
                return RepositoryResult::skipped(repository, "nothing to unstage");
            }
            let head_is_unborn = match git::head_is_unborn(path) {
                Ok(head_is_unborn) => head_is_unborn,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            let output = if head_is_unborn {
                // `git restore --staged` requires HEAD. Emptying the unborn index has the same
                // unstage-all result and leaves every working-tree file untouched.
                git::run_with_options(path, ["read-tree", "--empty"], true, execution)
            } else {
                git::run_with_options(path, ["restore", "--staged", "--", ":/"], true, execution)
            };
            match output {
                Ok(output) => RepositoryResult::from_git(
                    repository,
                    output,
                    "staged changes removed; working tree preserved",
                    false,
                ),
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            }
        },
    )
}

/// Resolve one standard repository selection and execute an index/local-history operation.
fn run_selected_local_operation<F>(
    arguments: SyncArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
    command: &str,
    operation: F,
) -> Result<i32>
where
    F: Fn(&RepositoryRecord, &Path) -> RepositoryResult + Sync + Send,
{
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let manifest = workspace::read(&root)?;
    let records = crate::selector::select(
        &manifest,
        &arguments.selectors,
        &arguments.matches,
        arguments.all,
    )?;
    let results =
        map_repository_results(&records, jobs, automation, command, &root, |repository| {
            let path = root.join(&repository.directory);
            if !git::is_repository(&path) {
                return RepositoryResult::failed(
                    repository,
                    "repository is not materialized; run sync or restore",
                );
            }
            operation(repository, &path)
        })?;
    print_selected_results(&results, verbose, automation, command, &root)
}

/// 解析用户选择后执行安全的 fast-forward-only pull。
fn pull(
    arguments: SyncArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let mut manifest = workspace::read(&root)?;
    let records = crate::selector::select(
        &manifest,
        &arguments.selectors,
        &arguments.matches,
        arguments.all,
    )?;
    run_pull(&root, &mut manifest, &records, jobs, verbose, automation)
}

/// 对选中仓库检查工作树、分支和 upstream 后执行 fast-forward pull。
pub(crate) fn run_pull(
    root: &Path,
    manifest: &mut Workspace,
    records: &[RepositoryRecord],
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    run_pull_named(root, manifest, records, jobs, verbose, automation, "pull")
}

/// Execute pull with the externally visible operation name supplied by a parent workflow.
pub(crate) fn run_pull_named(
    root: &Path,
    manifest: &mut Workspace,
    records: &[RepositoryRecord],
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
    command: &str,
) -> Result<i32> {
    let results = map_repository_results(records, jobs, automation, command, root, |repository| {
        let path = root.join(&repository.directory);
        if !git::is_repository(&path) {
            return RepositoryResult::failed(
                repository,
                "repository is not materialized; run sync or restore",
            );
        }
        let status = match git::status_summary(&path) {
            Ok(status) => status,
            Err(error) => return RepositoryResult::failed(repository, error.to_string()),
        };
        if status.branch.starts_with("(detached:") || status.branch == "(unborn)" {
            return RepositoryResult::failed(repository, "current HEAD is not a local branch");
        }
        if status.changes.total() != 0 {
            return RepositoryResult::failed(repository, "working tree is not clean");
        }
        if matches!(status.upstream, UpstreamSummary::None) {
            return RepositoryResult::failed(repository, "current branch has no upstream");
        }
        match git::run_with_options(
            &path,
            ["pull", "--ff-only"],
            true,
            git_execution_options(automation, false),
        ) {
            Ok(output) => RepositoryResult::from_git(
                repository,
                output,
                format!("branch {} updated with fast-forward only", status.branch),
                true,
            ),
            Err(error) => RepositoryResult::failed(repository, error.to_string()),
        }
    })?;

    let timestamp = now();
    let mut changed = false;
    for (selected, outcome) in records.iter().zip(&results) {
        if !outcome.was_synced() {
            continue;
        }
        if let Some(record) = manifest
            .repositories
            .iter_mut()
            .find(|record| record.name == selected.name)
        {
            record.synced_at = Some(timestamp.clone());
            changed = true;
        }
    }
    if changed {
        workspace::write(root, manifest)?;
    }
    crate::report::print_operation_summary(&results, verbose, automation, command, root)
}

/// 推送各仓库当前分支，默认不创建远端分支且永不 force push。
fn push(
    arguments: PushArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let manifest = workspace::read(&root)?;
    let records = crate::selector::select(
        &manifest,
        &arguments.selection.selectors,
        &arguments.selection.matches,
        arguments.selection.all,
    )?;
    let results =
        map_repository_results(&records, jobs, automation, "push", &root, |repository| {
            let path = root.join(&repository.directory);
            if !git::is_repository(&path) {
                return RepositoryResult::failed(
                    repository,
                    "repository is not materialized; run sync or restore",
                );
            }
            let status = match git::status_summary(&path) {
                Ok(status) => status,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            if status.branch.starts_with("(detached:") || status.branch == "(unborn)" {
                return RepositoryResult::failed(repository, "current HEAD is not a local branch");
            }
            let upstream_target = match git::upstream_push_target(&path, &status.branch) {
                Ok(target) => target,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };

            match status.upstream {
                UpstreamSummary::None if upstream_target.is_some() => {
                    let (remote, merge_ref) =
                        upstream_target.as_ref().expect("target checked above");
                    push_existing_upstream(
                        repository,
                        &path,
                        &status.branch,
                        remote,
                        merge_ref,
                        arguments.dry_run,
                        git_execution_options(automation, jobs == 1),
                    )
                }
                UpstreamSummary::None if !arguments.set_upstream => {
                    RepositoryResult::skipped(repository, "current branch has no upstream")
                }
                UpstreamSummary::None => {
                    let remote = arguments
                        .remote
                        .as_deref()
                        .unwrap_or(&repository.primary_remote);
                    let mut git_arguments = vec!["push"];
                    if arguments.dry_run {
                        git_arguments.push("--dry-run");
                    }
                    git_arguments.extend(["--set-upstream", remote, "HEAD"]);
                    match git::run_with_options(
                        &path,
                        git_arguments,
                        true,
                        git_execution_options(automation, jobs == 1),
                    ) {
                        Ok(output) => RepositoryResult::from_git(
                            repository,
                            output,
                            if arguments.dry_run {
                                format!(
                                    "would push {} to {remote} and configure upstream",
                                    status.branch
                                )
                            } else {
                                format!(
                                    "pushed {} to {remote} and configured upstream",
                                    status.branch
                                )
                            },
                            false,
                        ),
                        Err(error) => RepositoryResult::failed(repository, error.to_string()),
                    }
                }
                UpstreamSummary::UpToDate => {
                    RepositoryResult::skipped(repository, "nothing to push")
                }
                UpstreamSummary::Behind(count) => RepositoryResult::skipped(
                    repository,
                    format!("branch is behind upstream by {count} commit(s)"),
                ),
                UpstreamSummary::Diverged { ahead, behind } => RepositoryResult::failed(
                    repository,
                    format!("branch has diverged from upstream: ahead {ahead}, behind {behind}"),
                ),
                UpstreamSummary::Ahead(_) => {
                    let Some((remote, merge_ref)) = upstream_target.as_ref() else {
                        return RepositoryResult::failed(
                            repository,
                            "current branch has no configured upstream target",
                        );
                    };
                    push_existing_upstream(
                        repository,
                        &path,
                        &status.branch,
                        remote,
                        merge_ref,
                        arguments.dry_run,
                        git_execution_options(automation, jobs == 1),
                    )
                }
            }
        })?;
    print_push_summary(&results, verbose, automation, "push", &root)
}

/// 按已有 upstream 配置推送，先拒绝分叉等不安全状态。
fn push_existing_upstream(
    repository: &RepositoryRecord,
    path: &Path,
    branch: &str,
    remote: &str,
    merge_ref: &str,
    dry_run: bool,
    execution: GitExecutionOptions,
) -> RepositoryResult {
    let refspec = format!("HEAD:{merge_ref}");
    let mut git_arguments = vec!["push"];
    if dry_run {
        git_arguments.push("--dry-run");
    }
    git_arguments.extend([remote, refspec.as_str()]);
    match git::run_with_options(path, git_arguments, true, execution) {
        Ok(output) => RepositoryResult::from_git(
            repository,
            output,
            format!("pushed {branch}{}", if dry_run { " (dry run)" } else { "" }),
            false,
        ),
        Err(error) => RepositoryResult::failed(repository, error.to_string()),
    }
}

/// 在各仓库解析并安全切换目标分支，缺少分支时允许正常跳过。
fn checkout(
    arguments: CheckoutArgs,
    jobs: usize,
    _verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let current_feature_branch = settings::current_feature_branch()?;
    let feature_branch = if arguments.feature {
        match current_feature_branch.as_deref() {
            Some(branch) => Some(branch),
            None => return feature_branch_unset(automation, "checkout"),
        }
    } else {
        None
    };
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let manifest = workspace::read(&root)?;
    if arguments.create && arguments.from.is_none() && arguments.remote.is_some() {
        bail!("--remote requires --from when used with checkout -b");
    }
    let remote = if arguments.create && arguments.from.is_none() {
        None
    } else {
        settings::checkout_remote(arguments.remote)
    };
    let results = map_repository_results(
        &manifest.repositories,
        jobs,
        automation,
        "checkout",
        &root,
        |repository| {
            let path = root.join(&repository.directory);
            if !git::is_repository(&path) {
                return RepositoryResult::failed(
                    repository,
                    "repository is not materialized; run restore",
                );
            }
            let branch = arguments
                .branch
                .as_deref()
                .or(feature_branch)
                .unwrap_or(&repository.default_branch);
            if arguments.create {
                return match git::create_and_checkout_branch(
                    &path,
                    branch,
                    arguments.from.as_deref(),
                    remote.as_deref(),
                ) {
                    Ok(()) => RepositoryResult::success(
                        repository,
                        format!("created branch {branch}"),
                        false,
                    ),
                    Err(error) => RepositoryResult::failed(repository, error.to_string()),
                };
            }
            let selected_remote = if arguments.default {
                Some(repository.primary_remote.as_str())
            } else {
                remote.as_deref()
            };
            let target = match git::checkout_target(&path, branch, selected_remote) {
                Ok(target) => target,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            match target {
                CheckoutTarget::Missing => {
                    RepositoryResult::skipped(repository, format!("branch {branch} does not exist"))
                }
                CheckoutTarget::Ambiguous(matches) => RepositoryResult::failed(
                    repository,
                    format!("branch is ambiguous: {}", matches.join(", ")),
                ),
                CheckoutTarget::Local => match git::checkout_local(&path, branch) {
                    Ok(()) => {
                        RepositoryResult::success(repository, "checked out local branch", false)
                    }
                    Err(error) => RepositoryResult::failed(repository, error.to_string()),
                },
                CheckoutTarget::Remote(remote_branch) => {
                    match git::checkout_remote(&path, branch, &remote_branch) {
                        Ok(()) => RepositoryResult::success(
                            repository,
                            format!("created tracking branch from {remote_branch}"),
                            false,
                        ),
                        Err(error) => RepositoryResult::failed(repository, error.to_string()),
                    }
                }
            }
        },
    )?;
    let branches = manifest
        .repositories
        .iter()
        .map(|repository| {
            let path = root.join(&repository.directory);
            if !path.exists() {
                "(missing)".to_owned()
            } else if !git::is_repository(&path) {
                "(not-git)".to_owned()
            } else {
                git::current_branch_summary(&path).unwrap_or_else(|_| "(unknown)".to_owned())
            }
        })
        .collect::<Vec<_>>();
    let default_branches = manifest
        .repositories
        .iter()
        .map(|repository| repository.default_branch.clone())
        .collect::<Vec<_>>();
    print_checkout_summary(
        &results,
        &branches,
        &default_branches,
        current_feature_branch.as_deref(),
        automation,
        "checkout",
        &root,
    )
}

/// 将指定源分支合入每个仓库当前分支，可选先快进当前分支。
fn merge(
    arguments: MergeArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let merge_settings = merge_settings(&arguments)?;
    let update_current = merge_settings.update_current;
    let refresh_source = merge_settings.refresh_source;
    let MergeArgs {
        update_current: _,
        no_update_current: _,
        refresh_source: _,
        no_refresh_source: _,
        remote,
        default,
        feature,
        branch,
    } = arguments;
    let feature_branch = if feature {
        match settings::current_feature_branch()? {
            Some(branch) => Some(branch),
            None => return feature_branch_unset(automation, "merge"),
        }
    } else {
        None
    };
    let shared_branch = branch.as_deref().or(feature_branch.as_deref());
    let selected_remote = if default {
        None
    } else {
        settings::checkout_remote(remote)
    };
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let mut manifest = workspace::read(&root)?;
    let results = map_repository_results(
        &manifest.repositories,
        jobs,
        automation,
        "merge",
        &root,
        |repository| {
            let branch = if default {
                repository.default_branch.as_str()
            } else {
                shared_branch.expect("clap requires a branch, --feature, or --default")
            };
            let path = root.join(&repository.directory);
            if !git::is_repository(&path) {
                return RepositoryResult::failed(
                    repository,
                    "repository is not materialized; run restore",
                );
            }
            let status = match git::status_summary(&path) {
                Ok(status) => status,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            if status.branch.starts_with("(detached:") || status.branch == "(unborn)" {
                return RepositoryResult::failed(repository, "current HEAD is not a local branch");
            }
            if status.changes.total() != 0 {
                return RepositoryResult::failed(repository, "working tree is not clean");
            }
            if status.branch == branch {
                return RepositoryResult::skipped(repository, "source is the current branch");
            }

            if !update_current {
                match status.upstream {
                    UpstreamSummary::Behind(count) => {
                        return RepositoryResult::failed(
                            repository,
                            format!(
                                "current branch is behind upstream by {count} commit(s); run pull or retry merge with --update-current"
                            ),
                        );
                    }
                    UpstreamSummary::Diverged { ahead, behind } => {
                        return RepositoryResult::failed(
                            repository,
                            format!(
                                "current branch has diverged from upstream: ahead {ahead}, behind {behind}; resolve the divergence before merging"
                            ),
                        );
                    }
                    _ => {}
                }
            }

            let mut current_updated = false;
            if update_current && !matches!(status.upstream, UpstreamSummary::None) {
                let output = match git::run_with_options(
                    &path,
                    ["pull", "--ff-only"],
                    true,
                    git_execution_options(automation, jobs == 1),
                ) {
                    Ok(output) => output,
                    Err(error) => return RepositoryResult::failed(repository, error.to_string()),
                };
                if !output.success {
                    return RepositoryResult::from_git(
                        repository,
                        output,
                        "current branch updated",
                        false,
                    );
                }
                current_updated = true;
            }

            let remote = if default {
                Some(repository.primary_remote.as_str())
            } else {
                selected_remote.as_deref()
            };
            let source = if refresh_source {
                if let Err(error) = git::configure_declared_remotes(
                    &path,
                    repository,
                    git_execution_options(automation, jobs == 1).allow_stdin,
                ) {
                    return RepositoryResult::failed(repository, error.to_string());
                }
                let output = match git::fetch_all_with_options(
                    &path,
                    git_execution_options(automation, jobs == 1),
                ) {
                    Ok(output) => output,
                    Err(error) => return RepositoryResult::failed(repository, error.to_string()),
                };
                if !output.success {
                    return RepositoryResult::from_git(
                        repository,
                        output,
                        "source remote refs refreshed",
                        false,
                    );
                }
                let refresh_remote = remote.or(Some(repository.primary_remote.as_str()));
                match git::remote_tracking_target(&path, branch, refresh_remote) {
                    Ok(CheckoutTarget::Remote(remote_branch)) => remote_branch,
                    Ok(CheckoutTarget::Missing) => {
                        return RepositoryResult::skipped(
                            repository,
                            format!("remote source branch {branch} does not exist"),
                        );
                    }
                    Ok(CheckoutTarget::Ambiguous(matches)) => {
                        return RepositoryResult::failed(
                            repository,
                            format!("branch is ambiguous: {}", matches.join(", ")),
                        );
                    }
                    Ok(CheckoutTarget::Local) => {
                        unreachable!("remote-only resolution cannot be local")
                    }
                    Err(error) => return RepositoryResult::failed(repository, error.to_string()),
                }
            } else {
                match git::checkout_target(&path, branch, remote) {
                    Ok(CheckoutTarget::Local) => branch.to_owned(),
                    Ok(CheckoutTarget::Remote(remote_branch)) => remote_branch,
                    Ok(CheckoutTarget::Missing) => {
                        return RepositoryResult::skipped(
                            repository,
                            format!("branch {branch} does not exist"),
                        );
                    }
                    Ok(CheckoutTarget::Ambiguous(matches)) => {
                        return RepositoryResult::failed(
                            repository,
                            format!("branch is ambiguous: {}", matches.join(", ")),
                        );
                    }
                    Err(error) => return RepositoryResult::failed(repository, error.to_string()),
                }
            };
            match git::run_with_options(
                &path,
                ["merge", "--no-edit", source.as_str()],
                true,
                git_execution_options(automation, jobs == 1),
            ) {
                Ok(output) => RepositoryResult::from_git(
                    repository,
                    output,
                    format!("merged {source} into {}", status.branch),
                    current_updated || refresh_source,
                ),
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            }
        },
    )?;
    for (record, outcome) in manifest.repositories.iter_mut().zip(&results) {
        if outcome.was_synced() {
            record.synced_at = Some(now());
        }
    }
    if results.iter().any(RepositoryResult::was_synced) {
        workspace::write(&root, &mut manifest)?;
    }
    print_selected_results(&results, verbose, automation, "merge", &root)
}

/// Render an invocation-level no-op without pretending it is a per-repository batch result.
fn feature_branch_unset(automation: &AutomationOptions, operation: &str) -> Result<i32> {
    let data = json!({
        "status": "skipped",
        "reason_code": "feature_branch_unset",
        "detail": format!("CURRENT_FEATURE_BRANCH is not set; nothing to {operation}."),
    });
    if automation.is_machine() {
        automation::emit_data(automation, operation, None, 0, &data)?;
    } else {
        println!("CURRENT_FEATURE_BRANCH is not set; nothing to {operation}");
    }
    Ok(0)
}

/// 列出登记仓库及其实时当前分支，支持稳定 JSON 输出。
fn list(arguments: ListArgs, jobs: usize, automation: &AutomationOptions) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let repositories: Vec<ListRepository<'_>> = manifest
        .repositories
        .iter()
        .map(|repository| {
            let path = root.join(&repository.directory);
            let materialized = git::is_repository(&path);
            ListRepository {
                name: &repository.name,
                directory: &repository.directory,
                default_branch: &repository.default_branch,
                current_branch: materialized
                    .then(|| git::current_branch_summary(&path).ok())
                    .flatten(),
                materialized,
                synced_at: repository.synced_at.as_deref(),
            }
        })
        .collect();
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
        let rows: Vec<Vec<String>> = output
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
            .collect();
        print!("{}", table::render(&["NAME", "CURRENT", "DEFAULT"], &rows));
        println!();
        println!("{} repositories", manifest.repositories.len());
    }
    Ok(0)
}

#[derive(Clone, Copy, PartialEq, Eq)]
/// 工作区状态表中比 Git 原始状态更高层的分类。
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

/// 一个仓库在 status 表格中的完整行模型。
struct WorkspaceStatusRow {
    repository: String,
    branch: String,
    default_branch: String,
    kind: WorkspaceStatusKind,
    changes: String,
    upstream: String,
    changed_paths: usize,
}

/// 并行读取所有仓库状态，并按清单顺序输出紧凑摘要。
fn status(
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
    if automation.is_machine() {
        let data = statuses
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
            .collect::<Vec<_>>();
        automation::emit_data(
            automation,
            "status",
            Some(&root),
            exit_code,
            &json!({"repositories": data}),
        )?;
        return Ok(exit_code);
    }
    if arguments.json {
        let data = statuses
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
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"repositories": data}))?
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

/// 在已有本地引用中搜索分支，不隐式访问网络。
fn find(arguments: FindArgs, jobs: usize, automation: &AutomationOptions) -> Result<i32> {
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

/// 展示工作区整体或单仓库的清单与实时 Git 元数据。
fn info(arguments: InfoArgs, jobs: usize, automation: &AutomationOptions) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let current_feature_branch = settings::current_feature_branch()?;
    let Some(selector) = arguments.repository.as_deref() else {
        let runtime = map_ordered(&manifest.repositories, jobs, |repository| {
            git::repository_runtime_info(&root.join(&repository.directory))
        })?;
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
        return Ok(0);
    };

    let matches = manifest
        .repositories
        .iter()
        .filter(|repository| repository.name == selector || repository.directory == selector)
        .collect::<Vec<_>>();
    let repository = match matches.as_slice() {
        [repository] => *repository,
        [] => bail!("unknown repository selector: {selector}"),
        _ => bail!("ambiguous repository selector: {selector}"),
    };
    let path = root.join(&repository.directory);
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

/// 以人类可读格式打印单仓库详情。
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

/// 根据仓库可用性为状态文本着色。
fn color_runtime_state(state: &str) -> String {
    match state {
        "available" => color::green(state),
        "missing" | "not-git" | "bare" | "error" => color::red(state),
        _ => color::yellow(state),
    }
}

/// 命令模块的兼容包装，实际匹配规则由 selector 统一实现。
fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let value = value.chars().collect::<Vec<_>>();
    let mut previous = vec![false; value.len() + 1];
    previous[0] = true;
    for token in pattern {
        let mut current = vec![false; value.len() + 1];
        if token == '*' {
            current[0] = previous[0];
            for index in 1..=value.len() {
                current[index] = previous[index] || current[index - 1];
            }
        } else {
            for index in 1..=value.len() {
                current[index] = previous[index - 1] && value[index - 1] == token;
            }
        }
        previous = current;
    }
    previous[value.len()]
}

/// 快速展示每个仓库当前分支，不读取远端或工作树状态。
fn branch(
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
    let rows: Vec<Vec<String>> = states
        .into_iter()
        .map(|(name, default_branch, state)| {
            vec![
                name,
                color::branch(&state, &default_branch, feature_branch.as_deref()),
            ]
        })
        .collect();
    print!("{}", table::render(&["REPOSITORY", "BRANCH"], &rows));
    Ok(0)
}

/// 只从清单移除登记，绝不删除仓库目录和 Git 数据。
fn forget(arguments: ForgetArgs, automation: &AutomationOptions) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let mut manifest = workspace::read(&root)?;
    let mut indexes = HashSet::new();
    for selector in &arguments.selectors {
        let matches: Vec<usize> = manifest
            .repositories
            .iter()
            .enumerate()
            .filter_map(|(index, repository)| {
                (repository.name == *selector || repository.directory == *selector).then_some(index)
            })
            .collect();
        match matches.as_slice() {
            [index] => {
                indexes.insert(*index);
            }
            [] => bail!("unknown repository selector: {selector}"),
            _ => bail!("ambiguous repository selector: {selector}"),
        }
    }
    let removed: Vec<String> = manifest
        .repositories
        .iter()
        .enumerate()
        .filter(|(index, _)| indexes.contains(index))
        .map(|(_, repository)| repository.name.clone())
        .collect();
    manifest.repositories = manifest
        .repositories
        .into_iter()
        .enumerate()
        .filter_map(|(index, repository)| (!indexes.contains(&index)).then_some(repository))
        .collect();
    workspace::write(&root, &mut manifest)?;
    if automation.is_machine() {
        automation::emit_data(
            automation,
            "forget",
            Some(&root),
            0,
            &json!({"removed": removed, "directories_deleted": false}),
        )?;
    } else {
        for name in removed {
            println!("forgot {name}; repository directory was not deleted");
        }
    }
    Ok(0)
}

/// 从 URL 或路径末段推导默认 clone 目录，并移除 `.git` 后缀。
fn default_clone_directory(repository: &str) -> String {
    let trimmed = repository.trim_end_matches('/').trim_end_matches(".git");
    trimmed
        .rsplit(['/', ':'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("repository")
        .to_owned()
}

/// Atomically reserve an empty clone destination without deleting any user-controlled path.
///
/// Git accepts an existing empty destination. Reserving it with `create_dir` closes the
/// check-then-create race while deliberately leaving failed clone contents for manual review.
fn reserve_clone_destination(target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("clone destination has no parent: {}", target.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create clone parent {}", parent.display()))?;
    match fs::create_dir(target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            bail!("clone destination already exists: {}", target.display())
        }
        Err(error) => Err(error)
            .with_context(|| format!("failed to reserve clone destination {}", target.display())),
    }
}

/// 将工作区相对路径转换为清单使用的正斜杠字符串。
fn relative_string(path: &Path) -> Result<String> {
    if path.is_absolute() {
        bail!("directory must be relative to the workspace");
    }
    let parts: Vec<String> = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    Ok(parts.join("/"))
}

/// 在仓库名冲突时使用目录信息生成稳定、唯一的登记名称。
fn unique_name(base: &str, directory: &str, repositories: &[RepositoryRecord]) -> String {
    let used: HashSet<&str> = repositories
        .iter()
        .map(|repository| repository.name.as_str())
        .collect();
    if !used.contains(base) {
        return base.to_owned();
    }
    let candidate = directory.replace('/', "-");
    if !used.contains(candidate.as_str()) {
        return candidate;
    }
    for suffix in 2.. {
        let candidate = format!("{}-{suffix}", directory.replace('/', "-"));
        if !used.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!()
}

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

#[cfg(test)]
mod tests {
    use super::wildcard_matches;

    #[test]
    fn wildcard_matches_zero_or_more_unicode_characters() {
        assert!(wildcard_matches("main", "main"));
        assert!(!wildcard_matches("main", "main-old"));
        assert!(wildcard_matches("feature/*", "feature/login"));
        assert!(wildcard_matches("*登录*", "feature/登录-v2"));
        assert!(wildcard_matches("release/*/hotfix", "release/1.0/hotfix"));
        assert!(!wildcard_matches("release/*/hotfix", "release/hotfix"));
        assert!(wildcard_matches("**main**", "main"));
    }
}
