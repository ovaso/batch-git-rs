//! Black-box coverage for the versioned automation protocol.
//!
//! These tests deliberately exercise the compiled CLI. They keep the stable
//! protocol boundary separate from the human-oriented MVP smoke tests.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::cargo::CommandCargoExt;
use fs2::FileExt;
use serde_json::Value;
use tempfile::TempDir;

struct WorkspaceFixture {
    _temporary_directory: TempDir,
    workspace: PathBuf,
}

impl WorkspaceFixture {
    fn new() -> Self {
        let temporary_directory = tempfile::tempdir().expect("create temporary fixture");
        let root = temporary_directory.path();
        let remote = root.join("service.git");
        let seed = root.join("seed");
        let workspace = root.join("workspace");

        git(
            root,
            &[
                "init",
                "--bare",
                "--initial-branch=main",
                remote.to_str().expect("temporary path is UTF-8"),
            ],
        );
        git(
            root,
            &[
                "init",
                "--initial-branch=main",
                seed.to_str().expect("temporary path is UTF-8"),
            ],
        );
        git(&seed, &["config", "user.name", "batch-git protocol tests"]);
        git(
            &seed,
            &["config", "user.email", "batch-git@example.invalid"],
        );
        fs::write(seed.join("README.md"), "fixture\n").expect("write fixture file");
        git(&seed, &["add", "README.md"]);
        git(&seed, &["commit", "-m", "fixture"]);
        git(
            &seed,
            &[
                "remote",
                "add",
                "origin",
                remote.to_str().expect("temporary path is UTF-8"),
            ],
        );
        git(&seed, &["push", "-u", "origin", "main"]);

        fs::create_dir_all(&workspace).expect("create workspace directory");
        git(
            &workspace,
            &[
                "clone",
                remote.to_str().expect("temporary path is UTF-8"),
                "service",
            ],
        );
        write_manifest(&workspace, &remote);

        Self {
            _temporary_directory: temporary_directory,
            workspace,
        }
    }

    fn add_daily_sync_schedule(&self) {
        self.add_sync_schedule("skip");
    }

    fn add_queued_sync_schedule(&self) {
        self.add_sync_schedule("queue");
    }

    fn add_sync_schedule(&self, overlap: &str) {
        let manifest_path = self.workspace.join("workspace.toml");
        let mut manifest = fs::read_to_string(&manifest_path).expect("read fixture manifest");
        manifest.push_str(&format!(
            r#"

[[schedules]]
name = "nightly-sync"
action = "sync"
at = "02:30"
timezone = "local"
overlap = "{overlap}"

[schedules.scope]
all = true
"#,
        ));
        fs::write(manifest_path, manifest).expect("add fixture schedule");
    }
}

#[test]
fn output_json_list_wraps_the_legacy_list_payload() {
    let fixture = WorkspaceFixture::new();

    let legacy = json_output(run(&fixture.workspace, &["list", "--json"]));
    assert!(
        legacy.get("api_version").is_none(),
        "legacy JSON stays unwrapped"
    );
    assert!(legacy["repositories"].is_array());
    assert_eq!(legacy["repositories"][0]["name"], "service");

    let receipt = json_output(run(&fixture.workspace, &["--output", "json", "list"]));
    assert_success_receipt(&receipt, "list");
    assert_eq!(receipt["data"]["workspace"], legacy["workspace"]);
    assert_eq!(receipt["data"]["repositories"], legacy["repositories"]);
    assert_workspace_revision(&receipt);
}

#[test]
fn status_and_branch_json_remain_direct_machine_payloads() {
    let fixture = WorkspaceFixture::new();

    let status = json_output(run(&fixture.workspace, &["status", "--json"]));
    assert!(
        status.get("api_version").is_none(),
        "legacy JSON stays unwrapped"
    );
    assert_eq!(status["repositories"][0]["repository"], "service");
    assert!(status["repositories"][0]["state"].is_string());

    let branch = json_output(run(&fixture.workspace, &["branch", "--json"]));
    assert!(
        branch.get("api_version").is_none(),
        "legacy JSON stays unwrapped"
    );
    assert_eq!(branch["repositories"][0]["repository"], "service");
    assert_eq!(branch["repositories"][0]["branch"], "main");
}

#[test]
fn capabilities_is_discoverable_through_the_versioned_json_protocol() {
    let fixture = WorkspaceFixture::new();

    let receipt = json_output(run(
        &fixture.workspace,
        &["--output", "json", "capabilities"],
    ));
    assert_success_receipt(&receipt, "capabilities");
    assert!(
        receipt["data"].is_object() && !receipt["data"].as_object().unwrap().is_empty(),
        "capabilities must expose at least one discoverable value"
    );
}

#[test]
fn successful_json_receipts_always_include_an_explicit_null_error_field() {
    let temporary_directory = tempfile::tempdir().expect("create temporary fixture");

    let receipt = json_output(run(
        temporary_directory.path(),
        &["--output", "json", "capabilities"],
    ));
    assert_success_envelope(&receipt, "capabilities");
    assert_eq!(
        receipt.get("error"),
        Some(&Value::Null),
        "success receipts must retain the nullable error field rather than omitting it"
    );
}

#[test]
fn invalid_automation_options_are_structured_as_invalid_arguments() {
    let temporary_directory = tempfile::tempdir().expect("create temporary fixture");
    let cases: &[(&[&str], &str)] = &[
        (
            &[
                "--output",
                "json",
                "--timeout",
                "not-a-duration",
                "capabilities",
            ],
            "capabilities",
        ),
        (
            &["--output", "json", "--timeout", "0s", "capabilities"],
            "capabilities",
        ),
        (
            &["--output", "json", "--jobs", "0", "capabilities"],
            "capabilities",
        ),
        (&["--output", "json", "--plan", "status"], "status"),
        (
            &["--output", "json", "--plan", "schedule", "list"],
            "schedule list",
        ),
        (&["--output", "json", "merge", "--default", "main"], "merge"),
        (
            &["--output", "json", "merge", "--default", "--feature"],
            "merge",
        ),
        (
            &[
                "--output",
                "json",
                "merge",
                "--default",
                "--remote",
                "origin",
            ],
            "merge",
        ),
    ];

    for (arguments, command) in cases {
        let output = run(temporary_directory.path(), arguments);
        assert_eq!(
            output.status.code(),
            Some(2),
            "invalid invocation {arguments:?} must exit with code 2"
        );
        let receipt = json_output(output);
        assert_eq!(receipt["command"], *command);
        assert_eq!(receipt["exit_code"], 2);
        assert_eq!(receipt["ok"], false);
        assert_eq!(receipt["error"]["code"], "invalid_arguments");
    }
}

#[test]
fn invalid_request_id_is_not_reflected_in_a_machine_error_receipt() {
    let temporary_directory = tempfile::tempdir().expect("create temporary fixture");
    let output = run(
        temporary_directory.path(),
        &[
            "--output",
            "json",
            "--request-id",
            "line-one\nline-two",
            "capabilities",
        ],
    );

    assert_eq!(output.status.code(), Some(2));
    let receipt = json_output(output);
    assert_eq!(receipt["error"]["code"], "invalid_arguments");
    assert!(
        receipt.get("request_id").is_none(),
        "an invalid request ID must not be echoed into the machine protocol"
    );
}

#[test]
fn request_id_length_is_counted_in_unicode_characters_not_utf8_bytes() {
    let temporary_directory = tempfile::tempdir().expect("create temporary fixture");
    let request_id = "中".repeat(70);
    let arguments = [
        "--output",
        "json",
        "--request-id",
        request_id.as_str(),
        "capabilities",
    ];

    let receipt = json_output(run(temporary_directory.path(), &arguments));
    assert_success_receipt(&receipt, "capabilities");
    assert_eq!(receipt["request_id"], request_id);
}

#[test]
fn schedule_list_keeps_its_legacy_array_and_wraps_it_in_the_v1_envelope() {
    let fixture = WorkspaceFixture::new();
    fixture.add_daily_sync_schedule();

    let legacy = json_output(run(&fixture.workspace, &["schedule", "list", "--json"]));
    assert!(legacy.is_array(), "legacy schedule list stays an array");
    assert_eq!(legacy[0]["name"], "nightly-sync");

    let receipt = json_output(run(
        &fixture.workspace,
        &["--output", "json", "schedule", "list"],
    ));
    assert_success_envelope(&receipt, "schedule list");
    assert!(
        receipt["data"].is_array(),
        "the v1 schedule-list receipt keeps the declared array directly in data"
    );
    assert_eq!(receipt["data"], legacy);
}

#[test]
fn output_json_turns_an_unknown_command_into_a_structured_parameter_error() {
    let fixture = WorkspaceFixture::new();

    let output = run(
        &fixture.workspace,
        &["--output", "json", "unknown-automation-command"],
    );
    assert_eq!(output.status.code(), Some(2));
    let response = json_output(output);
    assert_eq!(response["api_version"], "v1");
    assert_eq!(response["command"], "unknown-automation-command");
    assert_eq!(response["exit_code"], 2);
    assert_eq!(response["ok"], false);
    assert!(response["data"].is_null());
    assert!(response["error"].is_object());
    assert!(response["error"]["code"].is_string());
    assert!(response["error"]["message"].is_string());
}

#[test]
fn empty_workspace_environment_is_reported_as_workspace_not_found() {
    let temporary_directory = tempfile::tempdir().expect("create temporary fixture");
    let empty_workspace = temporary_directory.path().join("empty-workspace");
    fs::create_dir(&empty_workspace).expect("create empty workspace directory");

    let output = batch_git(temporary_directory.path())
        .env("BATCH_GIT_WORKSPACE", &empty_workspace)
        .args(["--output", "json", "list"])
        .output()
        .expect("run batch-git");
    assert_eq!(output.status.code(), Some(2));
    let receipt = json_output(output);
    assert_eq!(receipt["command"], "list");
    assert_eq!(receipt["error"]["code"], "workspace_not_found");
}

#[test]
fn plan_sync_returns_a_workspace_revision_without_rewriting_the_manifest() {
    let fixture = WorkspaceFixture::new();
    let manifest_path = fixture.workspace.join("workspace.toml");
    let before = fs::read(&manifest_path).expect("read manifest before plan");

    let receipt = json_output(run(
        &fixture.workspace,
        &["--output", "json", "--plan", "sync"],
    ));
    assert_success_receipt(&receipt, "sync");
    assert_workspace_revision(&receipt);
    assert_eq!(
        fs::read(&manifest_path).expect("read manifest after plan"),
        before,
        "planning must not rewrite workspace.toml"
    );
}

#[test]
fn merge_default_plan_exposes_each_declared_source_without_rewriting_the_manifest() {
    let fixture = WorkspaceFixture::new();
    let manifest_path = fixture.workspace.join("workspace.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .expect("read manifest")
        .replace(
            "default_branch = \"main\"",
            "default_branch = \"release/2026.08/customer-a\"",
        );
    fs::write(&manifest_path, manifest).expect("write complex default branch");
    let before = fs::read(&manifest_path).expect("read manifest before merge plan");

    let receipt = json_output(run(
        &fixture.workspace,
        &["--output", "json", "--plan", "merge", "--default"],
    ));
    assert_success_receipt(&receipt, "merge");
    assert_workspace_revision(&receipt);
    assert_eq!(receipt["data"]["mode"], "plan");
    assert_eq!(receipt["data"]["risk"], "working_tree");
    assert_eq!(
        receipt["data"]["workspace_revision"],
        receipt["workspace"]["revision"]
    );
    let source = &receipt["data"]["selection"]["repositories"][0];
    assert_eq!(source["source_branch"], "release/2026.08/customer-a");
    assert_eq!(source["source_mode"], "workspace_default");
    assert_eq!(source["remote_fallback"], "origin");
    assert_eq!(
        fs::read(&manifest_path).expect("read manifest after merge plan"),
        before,
        "merge planning must not rewrite workspace.toml"
    );
}

#[test]
fn initial_workspace_scan_and_clone_plans_keep_actionable_selection_without_a_revision() {
    let temporary_directory = tempfile::tempdir().expect("create temporary fixture");
    let root = temporary_directory.path();
    let candidate = root.join("candidate");
    git(
        root,
        &[
            "init",
            "--initial-branch=main",
            candidate.to_str().expect("temporary path is UTF-8"),
        ],
    );

    let scan = json_output(run(root, &["--output", "json", "--plan", "scan"]));
    assert_success_receipt(&scan, "scan");
    assert_plan_has_no_workspace_revision(&scan);
    assert_eq!(
        scan["data"]["selection"]["repositories"][0]["directory"],
        "candidate"
    );
    assert_eq!(
        scan["data"]["selection"]["repositories"][0]["status"],
        "candidate"
    );

    let clone = json_output(run(
        root,
        &[
            "--output",
            "json",
            "--plan",
            "clone",
            "https://user:secret@example.invalid/team/planned.git",
            "planned",
        ],
    ));
    assert_success_receipt(&clone, "clone");
    assert_plan_has_no_workspace_revision(&clone);
    let selection = &clone["data"]["selection"]["repositories"][0];
    assert_eq!(selection["directory"], "planned");
    assert_eq!(
        selection["source"],
        "https://example.invalid/team/planned.git"
    );
    assert!(
        !selection["source"]
            .as_str()
            .expect("clone plan source is a string")
            .contains("secret"),
        "plan data must not expose URL credentials"
    );
    assert!(
        !root.join("workspace.toml").exists(),
        "initial workspace plans must not create a manifest"
    );
}

#[test]
fn failed_machine_clone_keeps_its_reserved_destination_for_inspection() {
    let temporary_directory = tempfile::tempdir().expect("create temporary fixture");
    let root = temporary_directory.path();
    let missing_remote = root.join("missing-remote.git");
    let destination = root.join("failed-clone");

    let output = run(
        root,
        &[
            "--output",
            "json",
            "clone",
            missing_remote.to_str().expect("temporary path is UTF-8"),
            "failed-clone",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let receipt = json_output(output);
    assert_eq!(receipt["command"], "clone");
    assert_eq!(receipt.get("error"), Some(&Value::Null));
    assert_eq!(receipt["data"]["result"]["directory"], "failed-clone");
    assert_eq!(receipt["data"]["result"]["status"], "failed");
    assert_eq!(receipt["data"]["result"]["synchronized"], false);
    assert!(
        destination.is_dir(),
        "a failed clone must retain its atomically reserved destination rather than recursively deleting it"
    );
    assert!(
        !root.join("workspace.toml").exists(),
        "a failed initial clone must not register or create a workspace manifest"
    );
}

#[test]
fn workspace_schema_keeps_defaulted_fields_optional() {
    let temporary_directory = tempfile::tempdir().expect("create temporary fixture");
    let receipt = json_output(run(
        temporary_directory.path(),
        &["--output", "json", "schema", "workspace"],
    ));
    assert_success_receipt(&receipt, "schema");
    let schema = &receipt["data"];

    assert_schema_requires(schema, &["version", "created_at", "updated_at"]);
    assert_schema_omits_required(schema, &["repositories", "schedules"]);

    let repository = &schema["$defs"]["repository"];
    assert_schema_requires(
        repository,
        &["name", "directory", "default_branch", "created_at"],
    );
    assert_schema_omits_required(repository, &["primary_remote", "remotes", "synced_at"]);

    let schedule = &schema["$defs"]["schedule"];
    assert_schema_requires(schedule, &["name", "scope"]);
    assert_schema_omits_required(schedule, &["enabled", "action", "timezone", "overlap"]);
}

#[test]
fn operation_result_schema_requires_the_nullable_error_field() {
    let temporary_directory = tempfile::tempdir().expect("create temporary fixture");
    let receipt = json_output(run(
        temporary_directory.path(),
        &["--output", "json", "schema", "operation-result"],
    ));
    assert_success_receipt(&receipt, "schema");
    let schema = &receipt["data"];

    assert_schema_requires(
        schema,
        &["api_version", "command", "exit_code", "ok", "data", "error"],
    );
    assert!(
        schema["properties"]["error"]["anyOf"]
            .as_array()
            .is_some_and(|variants| variants.iter().any(|variant| variant["type"] == "null")),
        "operation-result schema must accept the explicit error: null success form"
    );
}

#[test]
fn invalid_workspace_manifest_has_a_stable_error_code() {
    let temporary_directory = tempfile::tempdir().expect("create temporary fixture");
    fs::write(
        temporary_directory.path().join("workspace.toml"),
        r#"version = 2
created_at = "2026-01-01T00:00:00Z"
updated_at = "2026-01-01T00:00:00Z"
"#,
    )
    .expect("write invalid workspace manifest");

    let output = run(temporary_directory.path(), &["--output", "json", "list"]);
    assert_eq!(output.status.code(), Some(2));
    let receipt = json_output(output);
    assert_eq!(receipt["command"], "list");
    assert_eq!(receipt["error"]["code"], "workspace_manifest_invalid");
}

#[test]
fn plan_restore_uses_the_same_invoking_root_as_restore() {
    let fixture = WorkspaceFixture::new();
    let nested_repository = fixture.workspace.join("service");

    let output = run(
        &nested_repository,
        &["--output", "json", "--plan", "restore"],
    );
    assert_eq!(output.status.code(), Some(2));
    let response = json_output(output);
    assert_eq!(response["command"], "restore");
    assert_eq!(response["error"]["code"], "operation_failed");
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("restore requires workspace.toml")),
        "restore planning must reject a nested invocation instead of planning a parent workspace"
    );
}

#[test]
fn apply_rejects_a_stale_workspace_revision_as_a_structured_error() {
    let fixture = WorkspaceFixture::new();
    let manifest_path = fixture.workspace.join("workspace.toml");

    let plan = json_output(run(
        &fixture.workspace,
        &["--output", "json", "--plan", "sync"],
    ));
    let revision = plan["workspace"]["revision"]
        .as_str()
        .expect("plan receipt includes workspace revision")
        .to_owned();

    let mut changed_manifest = fs::read_to_string(&manifest_path).expect("read manifest");
    changed_manifest.push_str("\n# make the plan revision stale\n");
    fs::write(&manifest_path, &changed_manifest).expect("make workspace revision stale");

    let output = run(
        &fixture.workspace,
        &[
            "--output",
            "json",
            "--apply",
            "--expect-workspace-revision",
            &revision,
            "sync",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    let response = json_output(output);
    assert_eq!(response["api_version"], "v1");
    assert_eq!(response["command"], "sync");
    assert_eq!(response["exit_code"], 2);
    assert_eq!(response["ok"], false);
    assert!(response["error"].is_object());
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("workspace revision")),
        "the structured error should explain the stale plan"
    );
    assert_eq!(
        fs::read_to_string(&manifest_path).expect("read manifest after rejected apply"),
        changed_manifest,
        "a rejected apply must not rewrite workspace.toml"
    );
}

#[test]
fn native_run_apply_rejects_a_stale_revision_before_starting_its_child() {
    let fixture = WorkspaceFixture::new();
    fixture.add_daily_sync_schedule();
    let manifest_path = fixture.workspace.join("workspace.toml");

    let plan = json_output(run(
        &fixture.workspace,
        &["--output", "json", "--plan", "sync"],
    ));
    let revision = plan["workspace"]["revision"]
        .as_str()
        .expect("plan receipt includes workspace revision")
        .to_owned();
    let mut changed_manifest = fs::read_to_string(&manifest_path).expect("read manifest");
    changed_manifest.push_str("\n# make native-run apply stale\n");
    fs::write(&manifest_path, &changed_manifest).expect("make workspace revision stale");

    let output = run(
        &fixture.workspace,
        &[
            "--output",
            "json",
            "--apply",
            "--expect-workspace-revision",
            &revision,
            "schedule",
            "native-run",
            "nightly-sync",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    let receipt = json_output(output);
    assert_eq!(receipt["command"], "schedule native-run");
    assert_eq!(receipt["error"]["code"], "stale_workspace_revision");
    assert_eq!(
        fs::read_to_string(&manifest_path).expect("read manifest after rejected native-run"),
        changed_manifest,
        "a stale outer native-run invocation must not launch a child that rewrites the manifest"
    );
}

#[test]
fn native_run_apply_preserves_a_stale_error_detected_after_the_child_acquires_the_lock() {
    let fixture = WorkspaceFixture::new();
    fixture.add_queued_sync_schedule();
    let manifest_path = fixture.workspace.join("workspace.toml");

    let plan = json_output(run(
        &fixture.workspace,
        &["--output", "json", "--plan", "sync"],
    ));
    let revision = plan["workspace"]["revision"]
        .as_str()
        .expect("plan receipt includes workspace revision")
        .to_owned();

    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(fixture.workspace.join(".workspace.lock"))
        .expect("open workspace lock");
    lock.lock_exclusive().expect("hold workspace lock");

    let state_directory = fixture.workspace.join("native-run-state");
    let mut native_run = Command::cargo_bin("batch-git").expect("batch-git binary");
    native_run
        .current_dir(&fixture.workspace)
        .env_remove("BATCH_GIT_WORKSPACE")
        .env_remove("BATCH_GIT_JOBS")
        .env_remove("CURRENT_FEATURE_BRANCH")
        .env("NO_COLOR", "1")
        .env("BATCH_GIT_STATE_DIR", &state_directory)
        .args([
            "--output",
            "json",
            "--apply",
            "--expect-workspace-revision",
            &revision,
            "schedule",
            "native-run",
            "nightly-sync",
            "--log",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = native_run.spawn().expect("spawn native-run");

    if !wait_for_native_run_log(&state_directory, "nightly-sync", Duration::from_secs(5)) {
        let _ = FileExt::unlock(&lock);
        let _ = child.kill();
        let _ = child.wait();
        panic!("native-run did not reach its post-preflight logging boundary");
    }
    let mut changed_manifest = fs::read_to_string(&manifest_path).expect("read manifest");
    changed_manifest.push_str("\n# make child lock-time revision stale\n");
    fs::write(&manifest_path, &changed_manifest).expect("make workspace revision stale");
    FileExt::unlock(&lock).expect("release workspace lock");

    let output = child.wait_with_output().expect("wait for native-run");
    assert_eq!(output.status.code(), Some(2));
    let receipt = json_output(output);
    assert_eq!(receipt["command"], "schedule native-run");
    assert_eq!(receipt["error"]["code"], "stale_workspace_revision");
    assert_eq!(
        fs::read_to_string(&manifest_path).expect("read manifest after stale native-run"),
        changed_manifest,
        "the child must reject before it can rewrite the changed manifest"
    );
}

#[cfg(unix)]
#[test]
fn timed_out_git_alias_returns_without_waiting_for_a_pipe_holding_descendant() {
    let fixture = WorkspaceFixture::new();
    let repository = fixture.workspace.join("service");
    git(&repository, &["config", "alias.sleep", "!sleep 3"]);

    let started = Instant::now();
    let output = run(
        &fixture.workspace,
        &[
            "--output",
            "json",
            "--timeout",
            "1s",
            "exec",
            "service",
            "--",
            "sleep",
        ],
    );
    let elapsed = started.elapsed();

    assert_eq!(output.status.code(), Some(1));
    let receipt = json_output(output);
    assert_eq!(receipt["command"], "exec");
    assert_eq!(receipt.get("error"), Some(&Value::Null));
    assert_eq!(receipt["data"]["results"][0]["reason_code"], "timeout");
    assert!(
        elapsed < Duration::from_millis(2_500),
        "a one-second child timeout should not wait for the alias descendant to close inherited pipes; elapsed: {elapsed:?}"
    );
}

#[test]
fn jsonl_batch_and_schedule_runs_emit_a_complete_ordered_lifecycle() {
    let fixture = WorkspaceFixture::new();

    let fetch = run(&fixture.workspace, &["--output", "jsonl", "fetch"]);
    assert_eq!(fetch.status.code(), Some(0));
    assert_jsonl_repository_lifecycle(jsonl_output(fetch), "fetch");

    fixture.add_daily_sync_schedule();
    let scheduled = run(
        &fixture.workspace,
        &["--output", "jsonl", "schedule", "run", "nightly-sync"],
    );
    assert_eq!(scheduled.status.code(), Some(0));
    assert_jsonl_repository_lifecycle(jsonl_output(scheduled), "schedule run");
}

#[test]
fn jsonl_clone_emits_a_complete_single_repository_lifecycle() {
    let fixture = WorkspaceFixture::new();
    let remote = fixture
        .workspace
        .parent()
        .expect("workspace has a temporary parent")
        .join("service.git");
    let output = batch_git(&fixture.workspace)
        .args(["--output", "jsonl", "clone"])
        .arg(&remote)
        .arg("cloned-service")
        .output()
        .expect("run batch-git clone");

    assert_eq!(output.status.code(), Some(0));
    let events = jsonl_output(output);
    assert_eq!(
        events
            .iter()
            .map(|event| event["event"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["started", "repository_finished", "finished"],
        "clone must report its single repository before the aggregate completion event"
    );
    assert!(events.iter().all(|event| event["command"] == "clone"));
    assert_eq!(events[0]["data"]["repositories"], 1);
    assert_eq!(events[1]["data"]["repository_index"], 0);
    assert_eq!(events[1]["data"]["result"]["repository"], "cloned-service");
    assert_eq!(events[1]["data"]["result"]["directory"], "cloned-service");
    assert_eq!(events[1]["data"]["result"]["status"], "ok");
    assert_eq!(events[2]["data"]["result"], events[1]["data"]["result"]);
    assert_eq!(events[2]["exit_code"], 0);
    assert_eq!(events[2]["ok"], true);
}

fn run(workspace: &Path, arguments: &[&str]) -> Output {
    batch_git(workspace)
        .args(arguments)
        .output()
        .expect("run batch-git")
}

fn batch_git(workspace: &Path) -> assert_cmd::Command {
    let mut command =
        assert_cmd::Command::from_std(Command::cargo_bin("batch-git").expect("batch-git binary"));
    command
        .current_dir(workspace)
        .env_remove("BATCH_GIT_WORKSPACE")
        .env_remove("BATCH_GIT_JOBS")
        .env_remove("CURRENT_FEATURE_BRANCH")
        .env("NO_COLOR", "1");
    command
}

fn json_output(output: Output) -> Value {
    assert!(
        output.stderr.is_empty(),
        "machine output must not leak diagnostics to stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout must contain exactly one JSON document: {error}; stdout was: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn jsonl_output(output: Output) -> Vec<Value> {
    assert!(
        output.stderr.is_empty(),
        "machine output must not leak diagnostics to stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("machine output is UTF-8");
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("every JSONL line is valid JSON"))
        .collect::<Vec<Value>>();
    assert!(!events.is_empty(), "JSONL output must contain events");
    events
}

fn assert_jsonl_repository_lifecycle(events: Vec<Value>, command: &str) {
    assert_eq!(
        events
            .iter()
            .map(|event| event["event"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["started", "repository_finished", "finished"],
        "a one-repository batch must emit the complete lifecycle"
    );
    assert!(events.iter().all(|event| event["api_version"] == "v1"));
    assert!(events.iter().all(|event| event["command"] == command));
    assert_eq!(events[0]["data"]["repositories"], 1);
    assert_eq!(events[1]["data"]["repository_index"], 0);
    assert_eq!(events[1]["data"]["result"]["repository"], "service");
    assert_eq!(events[2]["exit_code"], 0);
    assert_eq!(events[2]["ok"], true);
}

fn wait_for_native_run_log(state_directory: &Path, schedule: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if fs::read_dir(state_directory.join("logs"))
            .ok()
            .is_some_and(|entries| {
                entries
                    .filter_map(Result::ok)
                    .any(|entry| entry.path().join(schedule).join("stdout.log").is_file())
            })
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn assert_success_receipt(receipt: &Value, command: &str) {
    assert_success_envelope(receipt, command);
    assert!(receipt["data"].is_object());
}

fn assert_success_envelope(receipt: &Value, command: &str) {
    assert_eq!(receipt["api_version"], "v1");
    assert_eq!(receipt["command"], command);
    assert_eq!(receipt["exit_code"], 0);
    assert_eq!(receipt["ok"], true);
    assert_eq!(
        receipt.get("error"),
        Some(&Value::Null),
        "success receipts must explicitly include error: null"
    );
}

fn assert_plan_has_no_workspace_revision(receipt: &Value) {
    assert!(receipt["workspace"]["path"].is_string());
    assert!(
        receipt["workspace"].get("revision").is_none(),
        "initial workspace plans must not invent a manifest revision"
    );
    assert!(receipt["data"]["workspace_revision"].is_null());
    assert_eq!(
        receipt["data"]["apply"]["requires_workspace_revision"], false,
        "an initial workspace plan must ask for direct explicit approval rather than an impossible --apply revision"
    );
}

fn assert_schema_requires(schema: &Value, fields: &[&str]) {
    let required = schema["required"]
        .as_array()
        .expect("schema object declares a required array");
    for field in fields {
        assert!(
            required
                .iter()
                .any(|required_field| required_field.as_str() == Some(field)),
            "schema must require {field}"
        );
    }
}

fn assert_schema_omits_required(schema: &Value, fields: &[&str]) {
    let required = schema["required"]
        .as_array()
        .expect("schema object declares a required array");
    for field in fields {
        assert!(
            !required
                .iter()
                .any(|required_field| required_field.as_str() == Some(field)),
            "schema must allow defaulted field {field} to be omitted"
        );
    }
}

fn assert_workspace_revision(receipt: &Value) {
    assert!(receipt["workspace"]["path"].is_string());
    assert!(
        receipt["workspace"]["revision"]
            .as_str()
            .is_some_and(|revision| revision.starts_with("sha256:")),
        "machine receipts must return the plan/apply workspace revision"
    );
}

fn write_manifest(workspace: &Path, remote: &Path) {
    let remote = toml_string(remote);
    let manifest = format!(
        r#"version = 1
created_at = "2026-01-01T00:00:00Z"
updated_at = "2026-01-01T00:00:00Z"

[[repositories]]
name = "service"
directory = "service"
default_branch = "main"
primary_remote = "origin"
created_at = "2026-01-01T00:00:00Z"

[[repositories.remotes]]
name = "origin"
fetch_url = "{remote}"
"#,
    );
    fs::write(workspace.join("workspace.toml"), manifest).expect("write fixture manifest");
}

fn toml_string(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run Git fixture command");
    assert!(
        output.status.success(),
        "Git fixture command failed: {arguments:?}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
