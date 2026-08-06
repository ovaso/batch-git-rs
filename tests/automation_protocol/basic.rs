use super::*;

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
    let commands = receipt["data"]["commands"]
        .as_array()
        .expect("capabilities commands are an array");
    let expected_commands = vec![
        "add",
        "branch",
        "capabilities",
        "checkout",
        "clone",
        "commit",
        "env",
        "exec",
        "fetch",
        "find",
        "forget",
        "info",
        "list",
        "merge",
        "pull",
        "push",
        "restore",
        "scan",
        "schema",
        "status",
        "sync",
        "unstage",
        "passthrough",
    ];
    #[cfg(feature = "schedule")]
    let expected_commands = {
        let mut expected_commands = expected_commands;
        expected_commands.insert(18, "schedule");
        expected_commands
    };
    assert_eq!(
        receipt["data"]["commands"],
        serde_json::json!(expected_commands),
        "internal command refactors must not rename or regroup automation command strings"
    );
    for command in ["add", "commit", "env", "unstage"] {
        assert!(
            commands.iter().any(|candidate| candidate == command),
            "capabilities must advertise the controlled {command} command"
        );
    }
    let safety = &receipt["data"]["safety"];
    assert_eq!(safety["commit_stages_content"], false);
    assert_eq!(safety["add_rejects_unresolved_conflicts"], true);
    assert_eq!(safety["commit_rejects_repository_operations"], true);
    assert_eq!(safety["unstage_preserves_working_trees"], true);
}

#[test]
fn env_list_exposes_effective_values_through_the_v1_protocol() {
    let fixture = WorkspaceFixture::new();
    let state_directory = fixture.workspace.join("env-state");
    let output = batch_git(&fixture.workspace)
        .env_remove("BATCH_GIT_SCAN_DEPTH")
        .env("BATCH_GIT_STATE_DIR", &state_directory)
        .env("BATCH_GIT_SCHEDULE_LOG", "yes")
        .env("BATCH_GIT_TZ", "Asia/Shanghai")
        .env("BATCH_GIT_REMOTE", "origin")
        .env("CURRENT_FEATURE_BRANCH", "feature/config")
        .env("BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE", "off")
        .env("BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT", "on")
        .env("BATCH_GIT_PASSTHROUGH_VERBOSE", "false")
        .args(["--output", "json", "--jobs", "9", "env", "ls"])
        .output()
        .expect("run env ls");
    let receipt = json_output(output);
    assert_success_receipt(&receipt, "env list");
    assert!(receipt["workspace"].is_null());

    let variables = receipt["data"]["variables"]
        .as_array()
        .expect("environment variables are an array");
    assert_eq!(variables.len(), 12);
    let variable = |name: &str| {
        variables
            .iter()
            .find(|variable| variable["name"] == name)
            .unwrap_or_else(|| panic!("missing environment variable {name}"))
    };

    assert_eq!(variable("BATCH_GIT_JOBS")["default"], "4");
    assert_eq!(variable("BATCH_GIT_JOBS")["current"], "9");
    let canonical_workspace = fixture
        .workspace
        .canonicalize()
        .expect("canonical workspace");
    assert_eq!(
        variable("BATCH_GIT_WORKSPACE")["current"],
        canonical_workspace.to_string_lossy().as_ref()
    );
    assert_eq!(
        variable("BATCH_GIT_STATE_DIR")["current"],
        state_directory.to_string_lossy().as_ref()
    );
    assert_eq!(variable("BATCH_GIT_SCHEDULE_LOG")["current"], "true");
    assert_eq!(variable("BATCH_GIT_TZ")["current"], "Asia/Shanghai");
    assert_eq!(variable("BATCH_GIT_REMOTE")["current"], "origin");
    assert_eq!(
        variable("CURRENT_FEATURE_BRANCH")["current"],
        "feature/config"
    );
    assert_eq!(
        variable("BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE")["current"],
        "false"
    );
    assert_eq!(
        variable("BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT")["current"],
        "true"
    );
    assert_eq!(
        variable("BATCH_GIT_PASSTHROUGH_VERBOSE")["current"],
        "false"
    );
    assert_eq!(variable("NO_COLOR")["current"], "true");
    assert!(variables.iter().all(|variable| {
        variable.get("description").is_none()
            && variable["default"].is_string()
            && variable["current"].is_string()
    }));

    let described = json_output(
        batch_git(&fixture.workspace)
            .env("BATCH_GIT_STATE_DIR", &state_directory)
            .args(["--output", "json", "env", "list", "-d"])
            .output()
            .expect("run described env list"),
    );
    assert_success_receipt(&described, "env list");
    assert!(
        described["data"]["variables"]
            .as_array()
            .is_some_and(|variables| variables
                .iter()
                .all(|variable| variable["description"].is_string()))
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
        (
            &["--output", "json", "commit", "--message", "   "],
            "commit",
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

#[cfg(feature = "schedule")]
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
fn selector_failures_keep_typed_machine_error_codes() {
    let fixture = WorkspaceFixture::new();
    let cases: &[(&[&str], &str)] = &[
        (
            &["--output", "json", "sync", "missing-repository"],
            "unknown_repository",
        ),
        (
            &["--output", "json", "sync", "--match", "missing-*"],
            "selector_no_match",
        ),
    ];

    for (arguments, expected_code) in cases {
        let output = run(&fixture.workspace, arguments);
        assert_eq!(output.status.code(), Some(2));
        let receipt = json_output(output);
        assert_eq!(receipt["error"]["code"], *expected_code);
    }
}
