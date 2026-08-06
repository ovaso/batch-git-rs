use super::*;

#[test]
fn sync_can_target_one_repository_or_the_whole_workspace() {
    let first = Fixture::new("sync-alpha");
    let second = Fixture::new("sync-beta");
    let workspace = tempfile::tempdir().unwrap();

    batch_git(workspace.path())
        .args([
            "clone",
            first.remote.to_str().unwrap(),
            "services/sync-alpha",
        ])
        .assert()
        .success();
    batch_git(workspace.path())
        .args([
            "clone",
            second.remote.to_str().unwrap(),
            "services/sync-beta",
        ])
        .assert()
        .success();

    fs::remove_dir_all(workspace.path().join("services/sync-alpha")).unwrap();
    batch_git(workspace.path())
        .args(["sync", "sync-alpha"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sync-alpha  ok"))
        .stdout(predicate::str::contains("sync-beta").not());
    assert_eq!(
        git_output(
            &workspace.path().join("services/sync-alpha"),
            ["branch", "--show-current"]
        ),
        "main"
    );

    batch_git(workspace.path())
        .args(["sync", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sync-alpha"))
        .stdout(predicate::str::contains("sync-beta"));

    batch_git(workspace.path())
        .args(["sync", "sync-alpha", "--all"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "--all cannot be combined with repository selectors",
        ));
}

#[test]
fn push_skips_missing_upstream_and_only_creates_it_when_requested() {
    let fixture = Fixture::new("push-service");
    let workspace = tempfile::tempdir().unwrap();
    batch_git(workspace.path())
        .args(["clone", fixture.remote.to_str().unwrap(), "push-service"])
        .assert()
        .success();

    let repository = workspace.path().join("push-service");
    git(&repository, ["config", "user.name", "Batch Git Tests"]);
    git(
        &repository,
        ["config", "user.email", "batch-git@example.invalid"],
    );
    git(&repository, ["checkout", "-b", "local-feature"]);
    fs::write(repository.join("LOCAL_FEATURE.md"), "first\n").unwrap();
    git(&repository, ["add", "LOCAL_FEATURE.md"]);
    git(&repository, ["commit", "-m", "local feature"]);

    batch_git(workspace.path())
        .args(["push"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "push-service  skipped  current branch has no upstream",
        ));
    assert!(!git_ref_exists(&fixture.remote, "refs/heads/local-feature"));

    batch_git(workspace.path())
        .args(["push", "--remote", "origin"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--set-upstream"));

    batch_git(workspace.path())
        .args(["push", "-u", "--dry-run"])
        .assert()
        .success();
    assert!(!git_ref_exists(&fixture.remote, "refs/heads/local-feature"));
    assert!(!git_succeeds(&repository, ["rev-parse", "@{upstream}"]));

    batch_git(workspace.path())
        .args(["push", "--set-upstream"])
        .assert()
        .success()
        .stdout(predicate::str::contains("push-service  ok"));
    assert!(git_ref_exists(&fixture.remote, "refs/heads/local-feature"));
    assert_eq!(
        git_output(&repository, ["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/local-feature"
    );

    let remote_before = git_output(
        workspace.path(),
        [
            "--git-dir",
            fixture.remote.to_str().unwrap(),
            "rev-parse",
            "refs/heads/local-feature",
        ],
    );
    fs::write(repository.join("LOCAL_FEATURE.md"), "second\n").unwrap();
    git(&repository, ["add", "LOCAL_FEATURE.md"]);
    git(&repository, ["commit", "-m", "second local feature"]);

    batch_git(workspace.path())
        .args(["push", "--dry-run"])
        .assert()
        .success();
    assert_eq!(
        git_output(
            workspace.path(),
            [
                "--git-dir",
                fixture.remote.to_str().unwrap(),
                "rev-parse",
                "refs/heads/local-feature",
            ],
        ),
        remote_before
    );

    batch_git(workspace.path())
        .args(["push"])
        .assert()
        .success();
    assert_eq!(
        git_output(
            workspace.path(),
            [
                "--git-dir",
                fixture.remote.to_str().unwrap(),
                "rev-parse",
                "refs/heads/local-feature",
            ],
        ),
        git_output(&repository, ["rev-parse", "HEAD"])
    );
}
