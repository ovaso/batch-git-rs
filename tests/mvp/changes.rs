use super::*;

#[test]
fn local_change_lifecycle_stages_unstages_and_commits_all_non_ignored_changes() {
    let fixture = Fixture::new("local-lifecycle");
    let workspace = tempfile::tempdir().unwrap();
    batch_git(workspace.path())
        .args(["clone", fixture.remote.to_str().unwrap(), "local-lifecycle"])
        .assert()
        .success();

    let repository = workspace.path().join("local-lifecycle");
    git(&repository, ["config", "user.name", "Batch Git Tests"]);
    git(
        &repository,
        ["config", "user.email", "batch-git@example.invalid"],
    );
    fs::write(repository.join(".gitignore"), "ignored.log\n").unwrap();
    fs::write(repository.join("DELETE_ME.md"), "delete me\n").unwrap();
    git(&repository, ["add", ".gitignore", "DELETE_ME.md"]);
    git(&repository, ["commit", "-m", "prepare staging fixture"]);

    let manifest_path = workspace.path().join("workspace.toml");
    let manifest_before = fs::read(&manifest_path).unwrap();
    let head_before = git_output(&repository, ["rev-parse", "HEAD"]);
    fs::write(repository.join("README.md"), "modified in working tree\n").unwrap();
    fs::write(repository.join("ADDED.md"), "new file\n").unwrap();
    fs::write(repository.join("ignored.log"), "ignored\n").unwrap();
    fs::remove_file(repository.join("DELETE_ME.md")).unwrap();

    batch_git(workspace.path())
        .args(["add"])
        .assert()
        .success()
        .stdout(predicate::str::contains("local-lifecycle  ok"));
    assert_eq!(git_output(&repository, ["rev-parse", "HEAD"]), head_before);
    let staged = git_output(&repository, ["diff", "--cached", "--name-status"]);
    assert!(staged.contains("A\tADDED.md"));
    assert!(staged.contains("D\tDELETE_ME.md"));
    assert!(staged.contains("M\tREADME.md"));
    assert!(!staged.contains("ignored.log"));

    batch_git(workspace.path())
        .args(["unstage"])
        .assert()
        .success()
        .stdout(predicate::str::contains("local-lifecycle  ok"));
    assert_eq!(
        git_output(&repository, ["ls-files", "--cached", "ADDED.md"]),
        ""
    );
    assert_eq!(
        git_output(&repository, ["diff", "--cached", "--name-only"]),
        ""
    );
    assert_eq!(
        fs::read_to_string(repository.join("README.md")).unwrap(),
        "modified in working tree\n"
    );
    assert!(repository.join("ADDED.md").is_file());
    assert!(!repository.join("DELETE_ME.md").exists());
    assert!(repository.join("ignored.log").is_file());
    assert_eq!(git_output(&repository, ["rev-parse", "HEAD"]), head_before);

    batch_git(workspace.path())
        .args(["unstage"])
        .assert()
        .success()
        .stdout(predicate::str::contains("local-lifecycle  skipped"))
        .stdout(predicate::str::contains(
            "summary: 0 ok, 1 skipped, 0 failed",
        ));

    batch_git(workspace.path()).args(["add"]).assert().success();
    batch_git(workspace.path())
        .args(["commit", "-m", "feat: safe local lifecycle"])
        .assert()
        .success()
        .stdout(predicate::str::contains("local-lifecycle  ok"));

    let head_after = git_output(&repository, ["rev-parse", "HEAD"]);
    assert_ne!(head_after, head_before);
    assert_eq!(
        git_output(&repository, ["log", "-1", "--format=%s"]),
        "feat: safe local lifecycle"
    );
    let committed = git_output(&repository, ["show", "--format=", "--name-status", "HEAD"]);
    assert!(committed.contains("A\tADDED.md"));
    assert!(committed.contains("D\tDELETE_ME.md"));
    assert!(committed.contains("M\tREADME.md"));
    assert!(!git_succeeds(
        &repository,
        ["cat-file", "-e", "HEAD:ignored.log"]
    ));
    assert_eq!(git_output(&repository, ["status", "--short"]), "");
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_before);

    batch_git(workspace.path())
        .args(["add"])
        .assert()
        .success()
        .stdout(predicate::str::contains("local-lifecycle  skipped"));
    batch_git(workspace.path())
        .args(["commit", "-m", "nothing should be created"])
        .assert()
        .success()
        .stdout(predicate::str::contains("local-lifecycle  skipped"));
    assert_eq!(git_output(&repository, ["rev-parse", "HEAD"]), head_after);
}

#[test]
fn local_change_commit_does_not_stage_unstaged_content() {
    let fixture = Fixture::new("commit-index-only");
    let workspace = tempfile::tempdir().unwrap();
    batch_git(workspace.path())
        .args([
            "clone",
            fixture.remote.to_str().unwrap(),
            "commit-index-only",
        ])
        .assert()
        .success();

    let repository = workspace.path().join("commit-index-only");
    git(&repository, ["config", "user.name", "Batch Git Tests"]);
    git(
        &repository,
        ["config", "user.email", "batch-git@example.invalid"],
    );
    fs::write(repository.join("README.md"), "staged snapshot\n").unwrap();
    git(&repository, ["add", "README.md"]);
    fs::write(repository.join("README.md"), "unstaged follow-up\n").unwrap();
    fs::write(repository.join("UNTRACKED.md"), "must stay untracked\n").unwrap();

    batch_git(workspace.path())
        .args(["commit", "-m", "commit the existing index"])
        .assert()
        .success()
        .stdout(predicate::str::contains("commit-index-only  ok"));
    assert_eq!(
        git_output(&repository, ["show", "HEAD:README.md"]),
        "staged snapshot"
    );
    assert_eq!(
        fs::read_to_string(repository.join("README.md")).unwrap(),
        "unstaged follow-up\n"
    );
    assert!(!git_succeeds(
        &repository,
        ["cat-file", "-e", "HEAD:UNTRACKED.md"]
    ));
    assert_eq!(
        git_output(&repository, ["ls-files", "--others", "--exclude-standard"]),
        "UNTRACKED.md"
    );

    let head = git_output(&repository, ["rev-parse", "HEAD"]);
    batch_git(workspace.path())
        .args(["commit", "-m", "must not stage remaining content"])
        .assert()
        .success()
        .stdout(predicate::str::contains("commit-index-only  skipped"))
        .stdout(predicate::str::contains(
            "summary: 0 ok, 1 skipped, 0 failed",
        ));
    assert_eq!(git_output(&repository, ["rev-parse", "HEAD"]), head);
    assert_eq!(
        git_output(&repository, ["diff", "--name-only"]),
        "README.md"
    );
}

#[test]
fn local_change_unstage_and_commit_support_an_unborn_branch() {
    let remote_directory = tempfile::tempdir().unwrap();
    let remote = remote_directory.path().join("empty.git");
    git(
        remote_directory.path(),
        [
            "init",
            "--bare",
            "--initial-branch=main",
            remote.to_str().unwrap(),
        ],
    );
    let workspace = tempfile::tempdir().unwrap();
    git(
        workspace.path(),
        ["clone", remote.to_str().unwrap(), "empty-service"],
    );
    batch_git(workspace.path())
        .args(["scan"])
        .assert()
        .success()
        .stdout(predicate::str::contains("added empty-service"));

    let repository = workspace.path().join("empty-service");
    fs::write(repository.join("FIRST.md"), "first commit content\n").unwrap();
    batch_git(workspace.path())
        .args(["add", "empty-service"])
        .assert()
        .success();
    assert_eq!(
        git_output(&repository, ["ls-files", "--cached"]),
        "FIRST.md"
    );
    assert!(!git_succeeds(
        &repository,
        ["rev-parse", "--verify", "HEAD"]
    ));

    batch_git(workspace.path())
        .args(["unstage", "empty-service"])
        .assert()
        .success()
        .stdout(predicate::str::contains("empty-service  ok"));
    assert_eq!(git_output(&repository, ["ls-files", "--cached"]), "");
    assert_eq!(
        git_output(&repository, ["ls-files", "--others", "--exclude-standard"]),
        "FIRST.md"
    );
    assert_eq!(
        fs::read_to_string(repository.join("FIRST.md")).unwrap(),
        "first commit content\n"
    );
    assert!(!git_succeeds(
        &repository,
        ["rev-parse", "--verify", "HEAD"]
    ));

    git(&repository, ["config", "user.name", "Batch Git Tests"]);
    git(
        &repository,
        ["config", "user.email", "batch-git@example.invalid"],
    );
    batch_git(workspace.path())
        .args(["add", "empty-service"])
        .assert()
        .success();
    batch_git(workspace.path())
        .args(["commit", "empty-service", "-m", "initial batch commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("empty-service  ok"));
    assert_eq!(
        git_output(&repository, ["log", "-1", "--format=%s"]),
        "initial batch commit"
    );
    assert_eq!(
        git_output(&repository, ["rev-list", "--count", "HEAD"]),
        "1"
    );
}

#[test]
fn local_change_unstage_does_not_confuse_an_unborn_named_branch_with_unborn_head() {
    let fixture = Fixture::new("unborn-named-branch");
    let workspace = tempfile::tempdir().unwrap();
    batch_git(workspace.path())
        .args([
            "clone",
            fixture.remote.to_str().unwrap(),
            "unborn-named-branch",
        ])
        .assert()
        .success();

    let repository = workspace.path().join("unborn-named-branch");
    git(&repository, ["config", "user.name", "Batch Git Tests"]);
    git(
        &repository,
        ["config", "user.email", "batch-git@example.invalid"],
    );
    fs::write(repository.join("KEEP.md"), "keep tracked\n").unwrap();
    git(&repository, ["add", "KEEP.md"]);
    git(&repository, ["commit", "-m", "add second tracked file"]);
    git(&repository, ["branch", "-m", "(unborn)"]);

    let head_before = git_output(&repository, ["rev-parse", "HEAD"]);
    fs::write(repository.join("README.md"), "modified but preserved\n").unwrap();
    git(&repository, ["add", "README.md"]);
    assert_eq!(
        git_output(&repository, ["diff", "--cached", "--name-only"]),
        "README.md"
    );

    batch_git(workspace.path())
        .args(["unstage", "unborn-named-branch"])
        .assert()
        .success()
        .stdout(predicate::str::contains("unborn-named-branch  ok"));

    assert_eq!(git_output(&repository, ["rev-parse", "HEAD"]), head_before);
    assert_eq!(
        git_output(&repository, ["diff", "--cached", "--name-only"]),
        ""
    );
    assert_eq!(
        git_output(&repository, ["diff", "--name-only"]),
        "README.md"
    );
    assert_eq!(
        git_output(&repository, ["ls-files", "--cached"]),
        "KEEP.md\nREADME.md"
    );
    assert_eq!(
        fs::read_to_string(repository.join("README.md")).unwrap(),
        "modified but preserved\n"
    );
}

#[test]
fn local_change_commands_share_repository_selection() {
    let first = Fixture::new("selection-alpha");
    let second = Fixture::new("selection-beta");
    let workspace = tempfile::tempdir().unwrap();
    batch_git(workspace.path())
        .args([
            "clone",
            first.remote.to_str().unwrap(),
            "services/selection-alpha",
        ])
        .assert()
        .success();
    batch_git(workspace.path())
        .args([
            "clone",
            second.remote.to_str().unwrap(),
            "apps/selection-beta",
        ])
        .assert()
        .success();

    let alpha = workspace.path().join("services/selection-alpha");
    let beta = workspace.path().join("apps/selection-beta");
    for repository in [&alpha, &beta] {
        git(repository, ["config", "user.name", "Batch Git Tests"]);
        git(
            repository,
            ["config", "user.email", "batch-git@example.invalid"],
        );
        fs::write(repository.join("README.md"), "selected change\n").unwrap();
    }

    batch_git(workspace.path())
        .args(["add", "selection-alpha"])
        .assert()
        .success()
        .stdout(predicate::str::contains("selection-beta").not());
    assert_eq!(
        git_output(&alpha, ["diff", "--cached", "--name-only"]),
        "README.md"
    );
    assert_eq!(git_output(&beta, ["diff", "--cached", "--name-only"]), "");

    batch_git(workspace.path())
        .args(["unstage", "--match", "selection-a*"])
        .assert()
        .success()
        .stdout(predicate::str::contains("selection-beta").not());
    assert_eq!(git_output(&alpha, ["diff", "--cached", "--name-only"]), "");

    batch_git(workspace.path())
        .args(["add", "--all"])
        .assert()
        .success();
    assert_eq!(
        git_output(&alpha, ["diff", "--cached", "--name-only"]),
        "README.md"
    );
    assert_eq!(
        git_output(&beta, ["diff", "--cached", "--name-only"]),
        "README.md"
    );

    let alpha_before = git_output(&alpha, ["rev-parse", "HEAD"]);
    let beta_before = git_output(&beta, ["rev-parse", "HEAD"]);
    batch_git(workspace.path())
        .args([
            "commit",
            "-m",
            "commit selected directory",
            "services/selection-alpha",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("selection-beta").not());
    assert_ne!(git_output(&alpha, ["rev-parse", "HEAD"]), alpha_before);
    assert_eq!(git_output(&beta, ["rev-parse", "HEAD"]), beta_before);
    assert_eq!(
        git_output(&beta, ["diff", "--cached", "--name-only"]),
        "README.md"
    );

    batch_git(workspace.path())
        .args(["commit", "-m", "commit the remaining default selection"])
        .assert()
        .success()
        .stdout(predicate::str::contains("selection-alpha  skipped"))
        .stdout(predicate::str::contains("selection-beta   ok"));
    assert_ne!(git_output(&beta, ["rev-parse", "HEAD"]), beta_before);
}

#[test]
fn local_change_commit_rejects_blank_messages() {
    let directory = tempfile::tempdir().unwrap();

    batch_git(directory.path())
        .args(["commit"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--message"));
    for message in ["", "   ", "\t"] {
        batch_git(directory.path())
            .args(["commit", "-m", message])
            .assert()
            .code(2)
            .stderr(predicate::str::contains("commit message cannot be empty"));
    }
}

#[test]
fn local_change_add_rejects_unresolved_conflicts() {
    let fixture = Fixture::new("add-conflict");
    let workspace = tempfile::tempdir().unwrap();
    batch_git(workspace.path())
        .args(["clone", fixture.remote.to_str().unwrap(), "add-conflict"])
        .assert()
        .success();

    let repository = workspace.path().join("add-conflict");
    git(&repository, ["config", "user.name", "Batch Git Tests"]);
    git(
        &repository,
        ["config", "user.email", "batch-git@example.invalid"],
    );
    git(&repository, ["checkout", "-b", "conflict-source"]);
    fs::write(repository.join("README.md"), "source side\n").unwrap();
    git(&repository, ["add", "README.md"]);
    git(&repository, ["commit", "-m", "source side"]);
    git(&repository, ["checkout", "main"]);
    fs::write(repository.join("README.md"), "current side\n").unwrap();
    git(&repository, ["add", "README.md"]);
    git(&repository, ["commit", "-m", "current side"]);

    let merge = Command::new("git")
        .current_dir(&repository)
        .args(["merge", "conflict-source"])
        .output()
        .expect("run conflicting merge");
    assert!(!merge.status.success());
    let unmerged_before = git_output(&repository, ["ls-files", "--unmerged"]);
    assert!(!unmerged_before.is_empty());

    batch_git(workspace.path())
        .args(["add", "add-conflict"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("unresolved conflicts"));
    assert_eq!(
        git_output(&repository, ["ls-files", "--unmerged"]),
        unmerged_before
    );
}

#[test]
fn local_change_commit_rejects_detached_head_and_repository_operations() {
    let fixture = Fixture::new("commit-safety");
    let workspace = tempfile::tempdir().unwrap();
    batch_git(workspace.path())
        .args(["clone", fixture.remote.to_str().unwrap(), "commit-safety"])
        .assert()
        .success();

    let repository = workspace.path().join("commit-safety");
    git(&repository, ["config", "user.name", "Batch Git Tests"]);
    git(
        &repository,
        ["config", "user.email", "batch-git@example.invalid"],
    );
    git(&repository, ["checkout", "--detach"]);
    fs::write(repository.join("DETACHED.md"), "detached\n").unwrap();
    git(&repository, ["add", "DETACHED.md"]);
    let detached_head = git_output(&repository, ["rev-parse", "HEAD"]);

    batch_git(workspace.path())
        .args(["commit", "-m", "must reject detached HEAD"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("HEAD is detached"));
    assert_eq!(
        git_output(&repository, ["rev-parse", "HEAD"]),
        detached_head
    );
    assert_eq!(
        git_output(&repository, ["ls-files", "--cached", "DETACHED.md"]),
        "DETACHED.md"
    );

    git(&repository, ["reset", "--hard"]);
    git(&repository, ["checkout", "main"]);
    git(&repository, ["checkout", "-b", "operation-source"]);
    fs::write(repository.join("OPERATION.md"), "merge content\n").unwrap();
    git(&repository, ["add", "OPERATION.md"]);
    git(&repository, ["commit", "-m", "operation source"]);
    git(&repository, ["checkout", "main"]);
    git(
        &repository,
        ["merge", "--no-commit", "--no-ff", "operation-source"],
    );
    let operation_head = git_output(&repository, ["rev-parse", "HEAD"]);

    batch_git(workspace.path())
        .args(["commit", "-m", "must reject merge continuation"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "repository operation is in progress",
        ));
    assert_eq!(
        git_output(&repository, ["rev-parse", "HEAD"]),
        operation_head
    );
    assert_eq!(
        git_output(&repository, ["diff", "--cached", "--name-only"]),
        "OPERATION.md"
    );
}
