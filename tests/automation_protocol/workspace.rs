use super::*;

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
        &["--output", "json", "--plan", "merge", "--default", "--rs"],
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
    assert_eq!(source["source_refresh_remote"], "origin");
    assert_eq!(
        receipt["data"]["parameters"],
        json!({"update_current": false, "refresh_source": true})
    );
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

#[cfg(unix)]
#[test]
fn workspace_manifest_rejects_repository_symlinks_outside_the_workspace() {
    use std::os::unix::fs::symlink;

    let fixture = WorkspaceFixture::new();
    let manifest_path = fixture.workspace.join("workspace.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .expect("read fixture manifest")
        .replace("directory = \"service\"", "directory = \"escape\"");
    fs::write(&manifest_path, manifest).expect("write escaped manifest path");
    let outside = fixture
        .workspace
        .parent()
        .expect("fixture workspace has a parent")
        .join("service.git");
    symlink(&outside, fixture.workspace.join("escape")).expect("create escaping symlink");

    let output = run(&fixture.workspace, &["--output", "json", "list"]);
    assert_eq!(output.status.code(), Some(2));
    let receipt = json_output(output);
    assert_eq!(receipt["error"]["code"], "workspace_manifest_invalid");
    assert!(
        receipt["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("resolves outside workspace"))
    );
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
