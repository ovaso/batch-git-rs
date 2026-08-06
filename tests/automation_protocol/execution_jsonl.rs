use super::*;

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

    #[cfg(feature = "schedule")]
    {
        fixture.add_daily_sync_schedule();
        let scheduled = run(
            &fixture.workspace,
            &["--output", "jsonl", "schedule", "run", "nightly-sync"],
        );
        assert_eq!(scheduled.status.code(), Some(0));
        assert_jsonl_repository_lifecycle(jsonl_output(scheduled), "schedule run");
    }
}

#[test]
fn local_change_noops_emit_complete_jsonl_batch_lifecycles() {
    let fixture = WorkspaceFixture::new();
    let cases = [
        (vec!["--output", "jsonl", "add"], "add", "nothing_to_stage"),
        (
            vec!["--output", "jsonl", "commit", "--message", "protocol no-op"],
            "commit",
            "nothing_to_commit",
        ),
        (
            vec!["--output", "jsonl", "unstage"],
            "unstage",
            "nothing_to_unstage",
        ),
    ];

    for (arguments, command, reason_code) in cases {
        let output = run(&fixture.workspace, &arguments);
        assert_eq!(output.status.code(), Some(0));
        let events = jsonl_output(output);
        assert_eq!(
            events[1]["data"]["result"]["reason_code"], reason_code,
            "{command} must retain its repository no-op reason in JSONL"
        );
        assert_eq!(events[2]["data"]["skipped"], 1);
        assert_jsonl_repository_lifecycle(events, command);
    }
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
