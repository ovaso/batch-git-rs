use super::*;

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
fn local_change_commands_emit_stable_json_receipts_and_noop_reasons() {
    let fixture = WorkspaceFixture::new();
    let repository = fixture.workspace.join("service");

    let add_noop = json_output(run(&fixture.workspace, &["--output", "json", "add"]));
    assert_single_repository_batch_result(&add_noop, "add", 0, "skipped", Some("nothing_to_stage"));

    let commit_noop = json_output(run(
        &fixture.workspace,
        &["--output", "json", "commit", "--message", "protocol no-op"],
    ));
    assert_single_repository_batch_result(
        &commit_noop,
        "commit",
        0,
        "skipped",
        Some("nothing_to_commit"),
    );

    let unstage_noop = json_output(run(&fixture.workspace, &["--output", "json", "unstage"]));
    assert_single_repository_batch_result(
        &unstage_noop,
        "unstage",
        0,
        "skipped",
        Some("nothing_to_unstage"),
    );

    fs::write(repository.join("README.md"), "staged by protocol test\n")
        .expect("modify tracked fixture file");
    fs::write(repository.join("ADDED.md"), "new staged content\n")
        .expect("write untracked fixture file");

    let add = json_output(run(&fixture.workspace, &["--output", "json", "add"]));
    assert_single_repository_batch_result(&add, "add", 0, "ok", None);

    let unstage = json_output(run(&fixture.workspace, &["--output", "json", "unstage"]));
    assert_single_repository_batch_result(&unstage, "unstage", 0, "ok", None);
    assert_eq!(
        git_stdout(&repository, &["diff", "--cached", "--name-only"]),
        "",
        "unstage must remove index changes before the commit workflow resumes"
    );
    assert_eq!(
        fs::read_to_string(repository.join("ADDED.md")).expect("read preserved worktree file"),
        "new staged content\n",
        "unstage must not remove working-tree content"
    );

    let add_again = json_output(run(&fixture.workspace, &["--output", "json", "add"]));
    assert_single_repository_batch_result(&add_again, "add", 0, "ok", None);
    configure_identity(&repository);
    let commit = json_output(run(
        &fixture.workspace,
        &[
            "--output",
            "json",
            "commit",
            "--message",
            "protocol local change",
        ],
    ));
    assert_single_repository_batch_result(&commit, "commit", 0, "ok", None);
    assert_eq!(
        git_stdout(&repository, &["log", "-1", "--format=%s"]),
        "protocol local change",
        "the controlled commit must forward one exact shared message"
    );
}

#[test]
fn local_change_plans_expose_parameters_without_mutating_repository_state() {
    let fixture = WorkspaceFixture::new();
    let repository = fixture.workspace.join("service");
    let manifest_path = fixture.workspace.join("workspace.toml");

    fs::write(repository.join("README.md"), "planned staged content\n")
        .expect("modify tracked fixture file");
    git(&repository, &["add", "README.md"]);
    fs::write(repository.join("UNSTAGED.md"), "planned worktree content\n")
        .expect("write unstaged fixture file");

    let manifest_before = fs::read(&manifest_path).expect("read manifest before plans");
    let head_before = git_stdout(&repository, &["rev-parse", "HEAD"]);
    let status_before = git_stdout(&repository, &["status", "--porcelain=v1"]);
    let staged_before = git_stdout(&repository, &["diff", "--cached", "--name-only"]);

    let add = json_output(run(
        &fixture.workspace,
        &["--output", "json", "--plan", "add"],
    ));
    assert_local_change_plan(&add, "add", "index", &["git_indexes"]);
    assert_eq!(
        add["data"]["parameters"],
        json!({
            "scope": "all_working_tree_changes",
            "includes": ["additions", "modifications", "deletions"],
            "force_ignored": false,
        })
    );

    let message = "reviewable protocol commit";
    let commit = json_output(run(
        &fixture.workspace,
        &["--output", "json", "--plan", "commit", "--message", message],
    ));
    assert_local_change_plan(
        &commit,
        "commit",
        "local_history",
        &["git_objects", "local_refs", "git_indexes", "hooks"],
    );
    assert_eq!(commit["data"]["parameters"]["message"], message);
    assert_eq!(commit["data"]["parameters"]["stages_content"], false);

    let unstage = json_output(run(
        &fixture.workspace,
        &["--output", "json", "--plan", "unstage"],
    ));
    assert_local_change_plan(&unstage, "unstage", "index", &["git_indexes"]);
    assert_eq!(
        unstage["data"]["parameters"],
        json!({
            "scope": "all_staged_changes",
            "preserves_working_tree": true,
            "moves_head": false,
        })
    );

    assert_eq!(
        fs::read(&manifest_path).expect("read manifest after plans"),
        manifest_before,
        "local-change planning must not rewrite workspace.toml"
    );
    assert_eq!(git_stdout(&repository, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(
        git_stdout(&repository, &["status", "--porcelain=v1"]),
        status_before,
        "planning must not stage, unstage, or commit content"
    );
    assert_eq!(
        git_stdout(&repository, &["diff", "--cached", "--name-only"]),
        staged_before,
        "planning must leave the index byte-for-byte equivalent at the protocol boundary"
    );
}

#[test]
fn add_and_commit_report_unresolved_conflicts_with_a_stable_reason_code() {
    let fixture = WorkspaceFixture::new();
    let repository = fixture.workspace.join("service");
    configure_identity(&repository);

    git(&repository, &["checkout", "-b", "conflict-side"]);
    fs::write(repository.join("README.md"), "side branch\n").expect("write side branch content");
    git(&repository, &["add", "README.md"]);
    git(&repository, &["commit", "-m", "side conflict"]);
    git(&repository, &["checkout", "main"]);
    fs::write(repository.join("README.md"), "main branch\n").expect("write main branch content");
    git(&repository, &["add", "README.md"]);
    git(&repository, &["commit", "-m", "main conflict"]);
    git_expect_failure(&repository, &["merge", "conflict-side"]);
    let head_before = git_stdout(&repository, &["rev-parse", "HEAD"]);
    let unmerged_before = git_stdout(&repository, &["ls-files", "--unmerged"]);

    let output = run(&fixture.workspace, &["--output", "json", "add"]);
    assert_eq!(output.status.code(), Some(1));
    let receipt = json_output(output);
    assert_single_repository_batch_result(
        &receipt,
        "add",
        1,
        "failed",
        Some("unresolved_conflicts"),
    );

    let output = run(
        &fixture.workspace,
        &[
            "--output",
            "json",
            "commit",
            "--message",
            "must not commit unresolved conflicts",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let receipt = json_output(output);
    assert_single_repository_batch_result(
        &receipt,
        "commit",
        1,
        "failed",
        Some("unresolved_conflicts"),
    );
    assert_eq!(
        git_stdout(&repository, &["rev-parse", "HEAD"]),
        head_before,
        "a rejected conflict commit must not move HEAD"
    );
    assert_eq!(
        git_stdout(&repository, &["ls-files", "--unmerged"]),
        unmerged_before,
        "a rejected conflict commit must leave the unmerged index untouched"
    );
    assert!(
        repository.join(".git").join("MERGE_HEAD").is_file(),
        "batch-git must leave conflict resolution to the explicit Git workflow"
    );
}

#[test]
fn commit_reports_detached_head_with_a_stable_reason_code() {
    let fixture = WorkspaceFixture::new();
    let repository = fixture.workspace.join("service");
    configure_identity(&repository);
    git(&repository, &["checkout", "--detach", "HEAD"]);
    fs::write(repository.join("DETACHED.md"), "detached content\n")
        .expect("write detached fixture content");
    git(&repository, &["add", "DETACHED.md"]);
    let head_before = git_stdout(&repository, &["rev-parse", "HEAD"]);

    let output = run(
        &fixture.workspace,
        &[
            "--output",
            "json",
            "commit",
            "--message",
            "must not commit detached",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let receipt = json_output(output);
    assert_single_repository_batch_result(&receipt, "commit", 1, "failed", Some("detached_head"));
    assert_eq!(
        git_stdout(&repository, &["rev-parse", "HEAD"]),
        head_before,
        "a rejected detached commit must not move HEAD"
    );
}

#[test]
fn commit_reports_repository_operations_in_progress_with_a_stable_reason_code() {
    let fixture = WorkspaceFixture::new();
    let repository = fixture.workspace.join("service");
    configure_identity(&repository);

    git(&repository, &["checkout", "-b", "pending-merge"]);
    fs::write(repository.join("PENDING.md"), "pending merge content\n")
        .expect("write pending merge fixture content");
    git(&repository, &["add", "PENDING.md"]);
    git(&repository, &["commit", "-m", "pending side"]);
    git(&repository, &["checkout", "main"]);
    git(
        &repository,
        &["merge", "--no-commit", "--no-ff", "pending-merge"],
    );
    let head_before = git_stdout(&repository, &["rev-parse", "HEAD"]);

    let output = run(
        &fixture.workspace,
        &[
            "--output",
            "json",
            "commit",
            "--message",
            "must not finish merge",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let receipt = json_output(output);
    assert_single_repository_batch_result(
        &receipt,
        "commit",
        1,
        "failed",
        Some("repository_operation_in_progress"),
    );
    assert_eq!(
        git_stdout(&repository, &["rev-parse", "HEAD"]),
        head_before,
        "a rejected in-progress operation must not create a commit"
    );
    assert!(
        repository.join(".git").join("MERGE_HEAD").is_file(),
        "batch-git must leave the explicit Git operation for the user to resolve"
    );
}
