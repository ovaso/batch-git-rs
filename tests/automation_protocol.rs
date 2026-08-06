//! Black-box coverage for the versioned automation protocol.
//!
//! These tests deliberately exercise the compiled CLI. They keep the stable
//! protocol boundary separate from the human-oriented MVP smoke tests.

use std::fs;
#[cfg(feature = "schedule")]
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
#[cfg(feature = "schedule")]
use std::process::Stdio;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use assert_cmd::cargo::CommandCargoExt;
#[cfg(feature = "schedule")]
use fs2::FileExt;
use serde_json::{Value, json};
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

    #[cfg(feature = "schedule")]
    fn add_daily_sync_schedule(&self) {
        self.add_sync_schedule("skip");
    }

    #[cfg(feature = "schedule")]
    fn add_queued_sync_schedule(&self) {
        self.add_sync_schedule("queue");
    }

    #[cfg(feature = "schedule")]
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

#[path = "automation_protocol/basic.rs"]
mod basic;
#[path = "automation_protocol/changes.rs"]
mod changes;
#[path = "automation_protocol/execution_jsonl.rs"]
mod execution_jsonl;
#[cfg(feature = "schedule")]
#[path = "automation_protocol/schedule.rs"]
mod schedule;
#[path = "automation_protocol/workspace.rs"]
mod workspace;

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

#[cfg(feature = "schedule")]
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

fn assert_single_repository_batch_result(
    receipt: &Value,
    command: &str,
    exit_code: i32,
    status: &str,
    reason_code: Option<&str>,
) {
    assert_eq!(receipt["api_version"], "v1");
    assert_eq!(receipt["command"], command);
    assert_eq!(receipt["exit_code"], exit_code);
    assert_eq!(receipt["ok"], exit_code == 0);
    assert_eq!(
        receipt.get("error"),
        Some(&Value::Null),
        "repository-level failures must keep the top-level error null"
    );
    assert_workspace_revision(receipt);

    let summary = &receipt["data"]["summary"];
    assert_eq!(summary["ok"], usize::from(status == "ok"));
    assert_eq!(summary["skipped"], usize::from(status == "skipped"));
    assert_eq!(summary["failed"], usize::from(status == "failed"));

    let results = receipt["data"]["results"]
        .as_array()
        .expect("batch receipt results are an array");
    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert_eq!(result["repository"], "service");
    assert_eq!(result["directory"], "service");
    assert_eq!(result["status"], status);
    assert_eq!(result["synchronized"], false);
    match reason_code {
        Some(reason_code) => assert_eq!(result["reason_code"], reason_code),
        None => assert!(
            result.get("reason_code").is_none(),
            "successful results must not invent a reason code"
        ),
    }
}

fn assert_local_change_plan(receipt: &Value, command: &str, risk: &str, side_effects: &[&str]) {
    assert_success_receipt(receipt, command);
    assert_workspace_revision(receipt);
    assert_eq!(receipt["data"]["mode"], "plan");
    assert_eq!(receipt["data"]["risk"], risk);
    assert_eq!(receipt["data"]["side_effects"], json!(side_effects));
    assert_eq!(
        receipt["data"]["workspace_revision"],
        receipt["workspace"]["revision"]
    );
    assert_eq!(
        receipt["data"]["selection"]["repositories"][0]["name"],
        "service"
    );
}

fn configure_identity(repository: &Path) {
    git(
        repository,
        &["config", "user.name", "batch-git protocol tests"],
    );
    git(
        repository,
        &["config", "user.email", "batch-git@example.invalid"],
    );
}

fn git_stdout(directory: &Path, arguments: &[&str]) -> String {
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
    String::from_utf8(output.stdout)
        .expect("Git fixture stdout is UTF-8")
        .trim_end()
        .to_owned()
}

fn git_expect_failure(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run failing Git fixture command");
    assert!(
        !output.status.success(),
        "Git fixture command unexpectedly succeeded: {arguments:?}"
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
