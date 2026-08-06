use super::*;

#[test]
fn merge_feature_setting_optionally_updates_the_current_branch_first() {
    let fixture = Fixture::new("service-merge");
    let workspace = tempfile::tempdir().unwrap();
    git(
        workspace.path(),
        ["clone", fixture.remote.to_str().unwrap(), "service-merge"],
    );
    batch_git(workspace.path())
        .args(["scan"])
        .assert()
        .success();
    let updater = tempfile::tempdir().unwrap();
    git(
        updater.path(),
        ["clone", fixture.remote.to_str().unwrap(), "updater"],
    );
    let updater_repository = updater.path().join("updater");
    git(
        &updater_repository,
        ["config", "user.name", "Batch Git Tests"],
    );
    git(
        &updater_repository,
        ["config", "user.email", "batch-git@example.invalid"],
    );
    fs::write(updater_repository.join("REMOTE.md"), "remote update\n").unwrap();
    git(&updater_repository, ["add", "REMOTE.md"]);
    git(&updater_repository, ["commit", "-m", "remote update"]);
    git(&updater_repository, ["push", "origin", "main"]);

    let repository = workspace.path().join("service-merge");
    git(&repository, ["config", "user.name", "Batch Git Tests"]);
    git(
        &repository,
        ["config", "user.email", "batch-git@example.invalid"],
    );
    batch_git(workspace.path())
        .args(["m", "feature"])
        .assert()
        .success()
        .stdout(predicate::str::contains("service-merge  ok"));
    assert!(repository.join("FEATURE.md").is_file());
    assert!(!repository.join("REMOTE.md").exists());

    git(&repository, ["checkout", "main"]);
    git(&repository, ["reset", "--hard", "origin/main"]);
    batch_git(workspace.path())
        .env("CURRENT_FEATURE_BRANCH", "feature")
        .env("BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT", "true")
        .args(["merge", "--feature"])
        .assert()
        .success()
        .stdout(predicate::str::contains("service-merge  ok"));
    assert!(repository.join("REMOTE.md").is_file());
    assert!(repository.join("FEATURE.md").is_file());
}

#[test]
fn merge_refresh_source_uses_the_latest_remote_tracking_branch() {
    let fixture = Fixture::new("service-merge-refresh-source");
    let workspace = tempfile::tempdir().unwrap();
    git(
        workspace.path(),
        [
            "clone",
            fixture.remote.to_str().unwrap(),
            "service-merge-refresh-source",
        ],
    );
    batch_git(workspace.path())
        .args(["scan"])
        .assert()
        .success();
    let repository = workspace.path().join("service-merge-refresh-source");
    git(&repository, ["checkout", "-b", "feature", "origin/feature"]);
    git(&repository, ["checkout", "main"]);

    let updater = tempfile::tempdir().unwrap();
    git(
        updater.path(),
        ["clone", fixture.remote.to_str().unwrap(), "updater"],
    );
    let updater_repository = updater.path().join("updater");
    git(
        &updater_repository,
        ["config", "user.name", "Batch Git Tests"],
    );
    git(
        &updater_repository,
        ["config", "user.email", "batch-git@example.invalid"],
    );
    git(&updater_repository, ["checkout", "feature"]);
    fs::write(
        updater_repository.join("REMOTE_FEATURE.md"),
        "latest feature update\n",
    )
    .unwrap();
    git(&updater_repository, ["add", "REMOTE_FEATURE.md"]);
    git(
        &updater_repository,
        ["commit", "-m", "latest feature update"],
    );
    git(&updater_repository, ["push", "origin", "feature"]);
    git(&updater_repository, ["checkout", "main"]);
    fs::write(
        updater_repository.join("REMOTE_TARGET.md"),
        "latest target update\n",
    )
    .unwrap();
    git(&updater_repository, ["add", "REMOTE_TARGET.md"]);
    git(
        &updater_repository,
        ["commit", "-m", "latest target update"],
    );
    git(&updater_repository, ["push", "origin", "main"]);

    assert!(!repository.join("REMOTE_FEATURE.md").exists());
    assert!(!repository.join("REMOTE_TARGET.md").exists());
    let local_source_before = git_output(&repository, ["rev-parse", "feature"]);
    batch_git(workspace.path())
        .args(["merge", "--uc", "--rs", "feature"])
        .assert()
        .success();
    assert!(repository.join("REMOTE_FEATURE.md").is_file());
    assert!(repository.join("REMOTE_TARGET.md").is_file());
    assert_eq!(
        git_output(&repository, ["rev-parse", "feature"]),
        local_source_before,
        "refreshing a source must not move its local branch"
    );
}

#[test]
fn merge_update_current_skips_a_local_only_target_branch() {
    let fixture = Fixture::new("service-merge-local-only-target");
    let workspace = tempfile::tempdir().unwrap();
    git(
        workspace.path(),
        [
            "clone",
            fixture.remote.to_str().unwrap(),
            "service-merge-local-only-target",
        ],
    );
    batch_git(workspace.path())
        .args(["scan"])
        .assert()
        .success();

    let repository = workspace.path().join("service-merge-local-only-target");
    git(&repository, ["checkout", "-b", "local-feature"]);
    assert!(
        !Command::new("git")
            .args(["config", "--get", "branch.local-feature.merge"])
            .current_dir(&repository)
            .status()
            .unwrap()
            .success(),
        "the local-only branch must not have an upstream"
    );

    batch_git(workspace.path())
        .args(["merge", "--uc", "feature"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "service-merge-local-only-target  ok",
        ));
    assert!(repository.join("FEATURE.md").is_file());
}

#[test]
fn merge_rejects_an_outdated_current_branch_without_starting_a_merge() {
    let fixture = Fixture::new("service-merge-outdated");
    let workspace = tempfile::tempdir().unwrap();
    git(
        workspace.path(),
        [
            "clone",
            fixture.remote.to_str().unwrap(),
            "service-merge-outdated",
        ],
    );
    batch_git(workspace.path())
        .args(["scan"])
        .assert()
        .success();

    let updater = tempfile::tempdir().unwrap();
    git(
        updater.path(),
        ["clone", fixture.remote.to_str().unwrap(), "updater"],
    );
    let updater_repository = updater.path().join("updater");
    git(
        &updater_repository,
        ["config", "user.name", "Batch Git Tests"],
    );
    git(
        &updater_repository,
        ["config", "user.email", "batch-git@example.invalid"],
    );
    fs::write(updater_repository.join("REMOTE.md"), "remote update\n").unwrap();
    git(&updater_repository, ["add", "REMOTE.md"]);
    git(&updater_repository, ["commit", "-m", "remote update"]);
    git(&updater_repository, ["push", "origin", "main"]);

    let repository = workspace.path().join("service-merge-outdated");
    git(&repository, ["fetch", "origin"]);
    let head_before = git_output(&repository, ["rev-parse", "HEAD"]);

    batch_git(workspace.path())
        .args(["merge", "feature"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("behind upstream by 1 commit(s)"));
    assert_eq!(git_output(&repository, ["rev-parse", "HEAD"]), head_before);
    assert!(
        !repository.join(".git").join("MERGE_HEAD").exists(),
        "the rejected merge must not leave an in-progress Git operation"
    );

    batch_git(workspace.path())
        .args(["pull"])
        .assert()
        .success();
    assert!(repository.join("REMOTE.md").is_file());
}

#[test]
fn merge_default_uses_each_declared_complex_branch_and_primary_remote() {
    let first_fixture = Fixture::new("service-default-one");
    let second_fixture = Fixture::new("service-default-two");
    let workspace = tempfile::tempdir().unwrap();
    git(
        workspace.path(),
        [
            "clone",
            first_fixture.remote.to_str().unwrap(),
            "service-default-one",
        ],
    );
    git(
        workspace.path(),
        [
            "clone",
            second_fixture.remote.to_str().unwrap(),
            "service-default-two",
        ],
    );
    batch_git(workspace.path())
        .args(["scan"])
        .assert()
        .success();

    let cases = [
        (
            &first_fixture,
            "service-default-one",
            "release/2026.08/customer-a",
            "DEFAULT_ONE.md",
        ),
        (
            &second_fixture,
            "service-default-two",
            "platform/stable/v2",
            "DEFAULT_TWO.md",
        ),
    ];
    for (fixture, name, default_branch, marker) in cases {
        let repository = workspace.path().join(name);
        git(&repository, ["config", "user.name", "Batch Git Tests"]);
        git(
            &repository,
            ["config", "user.email", "batch-git@example.invalid"],
        );
        git(&repository, ["checkout", "-b", default_branch]);
        fs::write(repository.join(marker), format!("{default_branch}\n")).unwrap();
        git(&repository, ["add", marker]);
        git(&repository, ["commit", "-m", "complex default branch"]);
        git(&repository, ["push", "-u", "origin", default_branch]);
        git(
            &repository,
            ["checkout", "-b", "feature/consume-default", "main"],
        );
        git(&repository, ["branch", "-D", default_branch]);
        git(
            &repository,
            ["remote", "add", "backup", fixture.remote.to_str().unwrap()],
        );
        git(&repository, ["fetch", "backup"]);
        set_default_branch(workspace.path(), name, default_branch);
    }

    batch_git(workspace.path())
        .env("BATCH_GIT_REMOTE", "missing-remote-must-be-ignored")
        .args(["merge", "--default", "--no-update-current"])
        .assert()
        .success()
        .stdout(predicate::str::contains("service-default-one  ok"))
        .stdout(predicate::str::contains("service-default-two  ok"))
        .stdout(predicate::str::contains(
            "summary: 2 ok, 0 skipped, 0 failed",
        ));

    let first = workspace.path().join("service-default-one");
    let second = workspace.path().join("service-default-two");
    for repository in [&first, &second] {
        assert_eq!(
            git_output(repository, ["branch", "--show-current"]),
            "feature/consume-default"
        );
    }
    assert!(first.join("DEFAULT_ONE.md").is_file());
    assert!(!first.join("DEFAULT_TWO.md").exists());
    assert!(second.join("DEFAULT_TWO.md").is_file());
    assert!(!second.join("DEFAULT_ONE.md").exists());

    batch_git(workspace.path())
        .args(["checkout", "--default"])
        .assert()
        .success();
    batch_git(workspace.path())
        .args(["merge", "--default", "--update-current"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "summary: 0 ok, 2 skipped, 0 failed",
        ));
}

#[test]
fn merge_default_help_and_conflicts_are_explicit() {
    let directory = tempfile::tempdir().unwrap();

    batch_git(directory.path())
        .args(["merge", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("-d, --default"))
        .stdout(predicate::str::contains("--update-current"))
        .stdout(predicate::str::contains("--uc"))
        .stdout(predicate::str::contains("--refresh-source"))
        .stdout(predicate::str::contains("--rs"))
        .stdout(predicate::str::contains("--no-refresh-source"))
        .stdout(predicate::str::contains("declared default branch"));

    for arguments in [
        vec!["merge", "--default", "main"],
        vec!["merge", "--default", "--feature"],
        vec!["merge", "--default", "--remote", "origin"],
    ] {
        batch_git(directory.path())
            .args(arguments)
            .assert()
            .code(2)
            .stderr(
                predicate::str::contains("--default")
                    .and(predicate::str::contains("cannot be used with")),
            );
    }
}

#[test]
fn merge_uses_batch_git_remote_to_disambiguate_non_default_sources() {
    let fixture = Fixture::new("service-merge-remote");
    let workspace = tempfile::tempdir().unwrap();
    git(
        workspace.path(),
        [
            "clone",
            fixture.remote.to_str().unwrap(),
            "service-merge-remote",
        ],
    );
    let repository = workspace.path().join("service-merge-remote");
    git(
        &repository,
        ["remote", "add", "backup", fixture.remote.to_str().unwrap()],
    );
    git(&repository, ["fetch", "backup"]);
    batch_git(workspace.path())
        .args(["scan"])
        .assert()
        .success();

    batch_git(workspace.path())
        .env_remove("BATCH_GIT_REMOTE")
        .args(["merge", "feature"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("branch is ambiguous"));

    batch_git(workspace.path())
        .env("BATCH_GIT_REMOTE", "origin")
        .args(["merge", "feature"])
        .assert()
        .success();
    assert!(repository.join("FEATURE.md").is_file());
}

#[test]
fn merge_feature_without_an_environment_value_is_a_no_op() {
    let directory = tempfile::tempdir().unwrap();

    batch_git(directory.path())
        .env_remove("CURRENT_FEATURE_BRANCH")
        .args(["merge", "--feature"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "CURRENT_FEATURE_BRANCH is not set; nothing to merge",
        ));

    batch_git(directory.path())
        .env("CURRENT_FEATURE_BRANCH", "feature")
        .args(["merge", "--feature", "main"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "the argument '--feature' cannot be used with '[BRANCH]'",
        ));
}

#[test]
fn merge_source_specific_settings_are_overridden_by_cli_options() {
    let workspace = tempfile::tempdir().unwrap();
    batch_git(workspace.path())
        .args(["scan"])
        .assert()
        .success();
    batch_git(workspace.path())
        .env("BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE", "invalid")
        .args(["merge", "--default"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "invalid BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE value",
        ));

    batch_git(workspace.path())
        .env("BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE", "invalid")
        .args(["merge", "--no-refresh-source", "--default"])
        .assert()
        .success();
    batch_git(workspace.path())
        .env("BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE", "invalid")
        .args(["merge", "--rs", "--default"])
        .assert()
        .success();

    batch_git(workspace.path())
        .env("BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT", "invalid")
        .env("CURRENT_FEATURE_BRANCH", "feature")
        .args(["merge", "--feature"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "invalid BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT value",
        ));

    batch_git(workspace.path())
        .env("BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT", "invalid")
        .env("CURRENT_FEATURE_BRANCH", "feature")
        .args(["merge", "--no-update-current", "--feature"])
        .assert()
        .success();
    batch_git(workspace.path())
        .env("BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT", "invalid")
        .env("CURRENT_FEATURE_BRANCH", "feature")
        .args(["merge", "--uc", "--feature"])
        .assert()
        .success();

    batch_git(workspace.path())
        .env("BATCH_GIT_MERGE_UPDATE_CURRENT", "invalid")
        .env("BATCH_GIT_MERGE_REFRESH_SOURCE", "invalid")
        .args(["merge", "feature"])
        .assert()
        .success();
}
