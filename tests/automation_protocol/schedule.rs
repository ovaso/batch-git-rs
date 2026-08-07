use super::*;

#[test]
fn invalid_schedule_requests_keep_the_typed_machine_error_code() {
    let fixture = WorkspaceFixture::new();
    let output = run(
        &fixture.workspace,
        &["--output", "json", "schedule", "status", "missing"],
    );
    assert_eq!(output.status.code(), Some(2));
    let receipt = json_output(output);
    assert_eq!(receipt["error"]["code"], "schedule_invalid");
}

#[test]
fn native_run_logs_execution_boundaries_and_performance_metadata() {
    let fixture = WorkspaceFixture::new();
    fixture.add_daily_sync_schedule();
    let state_directory = fixture.workspace.join("native-run-log-state");

    let output = batch_git(&fixture.workspace)
        .env("BATCH_GIT_STATE_DIR", &state_directory)
        .args(["schedule", "native-run", "nightly-sync", "--log"])
        .output()
        .expect("run logged native schedule");
    assert_eq!(output.status.code(), Some(0));

    let logs_root = state_directory.join("logs");
    let workspace_log_directory = fs::read_dir(&logs_root)
        .expect("read log root")
        .next()
        .expect("one workspace log directory")
        .expect("read workspace log directory")
        .path()
        .join("nightly-sync");
    for log_name in ["stdout.log", "stderr.log"] {
        let log =
            fs::read_to_string(workspace_log_directory.join(log_name)).expect("read schedule log");
        assert!(log.contains("event=started"));
        assert!(log.contains("action=sync"));
        assert!(log.contains("started_at="));
        assert!(log.contains("event=finished"));
        assert!(log.contains("finished_at="));
        assert!(log.contains("duration_ms="));
        assert!(log.contains("exit_code=0"));
    }
}

#[test]
fn legacy_lock_still_serializes_skip_schedules_during_upgrade() {
    let fixture = WorkspaceFixture::new();
    fixture.add_daily_sync_schedule();
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(fixture.workspace.join(".workspace.lock"))
        .expect("open legacy workspace lock");
    lock.lock_exclusive().expect("hold legacy workspace lock");

    let output = run(
        &fixture.workspace,
        &["--output", "json", "schedule", "run", "nightly-sync"],
    );
    FileExt::unlock(&lock).expect("release legacy workspace lock");
    assert_eq!(output.status.code(), Some(0));
    let receipt = json_output(output);
    assert_eq!(receipt["data"]["status"], "skipped");
    assert_eq!(receipt["data"]["reason_code"], "workspace_locked");
}

#[test]
fn native_run_apply_rejects_a_stale_revision_before_starting_its_child() {
    let fixture = WorkspaceFixture::new();
    fixture.add_daily_sync_schedule();
    let manifest_path = fixture.workspace.join("batchspace.toml");

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
    let manifest_path = fixture.workspace.join("batchspace.toml");

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
        .open(fixture.workspace.join(".batchspace.lock"))
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
