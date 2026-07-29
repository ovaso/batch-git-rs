use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::cargo::CommandCargoExt;
use predicates::prelude::*;
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    remote: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let temp = tempfile::tempdir().expect("temporary fixture");
        let remote = temp.path().join(format!("{name}.git"));
        let seed = temp.path().join("seed");
        git(
            temp.path(),
            [
                "init",
                "--bare",
                "--initial-branch=main",
                remote.to_str().unwrap(),
            ],
        );
        git(
            temp.path(),
            ["init", "--initial-branch=main", seed.to_str().unwrap()],
        );
        git(&seed, ["config", "user.name", "Batch Git Tests"]);
        git(&seed, ["config", "user.email", "batch-git@example.invalid"]);
        fs::write(seed.join("README.md"), "main\n").unwrap();
        git(&seed, ["add", "README.md"]);
        git(&seed, ["commit", "-m", "main"]);
        git(&seed, ["remote", "add", "origin", remote.to_str().unwrap()]);
        git(&seed, ["push", "-u", "origin", "main"]);
        git(&seed, ["checkout", "-b", "feature"]);
        fs::write(seed.join("FEATURE.md"), "feature\n").unwrap();
        git(&seed, ["add", "FEATURE.md"]);
        git(&seed, ["commit", "-m", "feature"]);
        git(&seed, ["push", "-u", "origin", "feature"]);
        git(&seed, ["checkout", "main"]);
        Self {
            _temp: temp,
            remote,
        }
    }
}

#[test]
fn scan_discovers_repositories_and_passthrough_finds_parent_workspace() {
    let fixture = Fixture::new("service-one");
    let workspace = tempfile::tempdir().unwrap();
    git(
        workspace.path(),
        ["clone", fixture.remote.to_str().unwrap(), "service-one"],
    );

    batch_git(workspace.path())
        .args(["scan"])
        .assert()
        .success()
        .stdout(predicate::str::contains("added service-one"));

    let manifest = fs::read_to_string(workspace.path().join("workspace.toml")).unwrap();
    assert!(manifest.contains("directory = \"service-one\""));
    assert!(manifest.contains("default_branch = \"main\""));
    assert!(!manifest.contains("branches"));

    batch_git(&workspace.path().join("service-one"))
        .args(["list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"materialized\": true"));

    batch_git(&workspace.path().join("service-one"))
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("main\n\n1 repositories"))
        .stdout(predicate::str::contains("DIRECTORY").not());

    batch_git(&workspace.path().join("service-one"))
        .args(["ls"])
        .assert()
        .success()
        .stdout(predicate::str::contains("main\n\n1 repositories"));

    batch_git(&workspace.path().join("service-one"))
        .args(["l", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"materialized\": true"));

    batch_git(&workspace.path().join("service-one"))
        .args(["-l", "--json"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unexpected argument '-l'"));

    batch_git(&workspace.path().join("service-one"))
        .args(["branch"])
        .assert()
        .success()
        .stdout(predicate::str::contains("REPOSITORY   BRANCH"))
        .stdout(predicate::str::contains("service-one  main"));

    batch_git(&workspace.path().join("service-one"))
        .args(["b"])
        .assert()
        .success()
        .stdout(predicate::str::contains("REPOSITORY   BRANCH"))
        .stdout(predicate::str::contains("service-one  main"));

    batch_git(&workspace.path().join("service-one"))
        .args(["s"])
        .assert()
        .success()
        .stdout(predicate::str::contains("REPOSITORY"))
        .stdout(predicate::str::contains("service-one"));

    batch_git(&workspace.path().join("service-one"))
        .env_remove("CURRENT_FEATURE_BRANCH")
        .args(["info"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ROOT"))
        .stdout(predicate::str::contains("REPOSITORIES  1"))
        .stdout(predicate::str::contains("MATERIALIZED  1"))
        .stdout(predicate::str::contains("FEATURE BRANCH").not());

    batch_git(&workspace.path().join("service-one"))
        .env("CURRENT_FEATURE_BRANCH", "feature/long-name")
        .args(["info"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "FEATURE BRANCH  feature/long-name",
        ));

    batch_git(&workspace.path().join("service-one"))
        .env("CURRENT_FEATURE_BRANCH", "feature/long-name")
        .args(["info", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"current_feature_branch\": \"feature/long-name\"",
        ));

    batch_git(&workspace.path().join("service-one"))
        .args(["i"])
        .assert()
        .success()
        .stdout(predicate::str::contains("REPOSITORIES  1"));

    batch_git(&workspace.path().join("service-one"))
        .args(["info", "service-one"])
        .assert()
        .success()
        .stdout(predicate::str::contains("STATE           available"))
        .stdout(predicate::str::contains("CURRENT BRANCH  main"))
        .stdout(predicate::str::contains(
            "BRANCHES        1 local, 2 remote",
        ))
        .stdout(predicate::str::contains("origin  yes"));

    batch_git(&workspace.path().join("service-one"))
        .args(["info", "service-one", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"state\": \"available\""))
        .stdout(predicate::str::contains("\"current_branch\": \"main\""))
        .stdout(predicate::str::contains("\"local_branches\": 1"))
        .stdout(predicate::str::contains("\"remote_branches\": 2"));

    batch_git(&workspace.path().join("service-one"))
        .args(["find", "*"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "summary: 3 branches in 1 repository; 1 local, 2 remote",
        ))
        .stdout(predicate::str::contains("HEAD").not());

    batch_git(&workspace.path().join("service-one"))
        .args(["fd", "feature*", "--remote"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "service-one  remote  origin  no       feature",
        ))
        .stdout(predicate::str::contains(
            "summary: 1 branch in 1 repository; 0 local, 1 remote",
        ));

    batch_git(&workspace.path().join("service-one"))
        .args(["f", "main", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\": \"main\""))
        .stdout(predicate::str::contains("\"kind\": \"local\""))
        .stdout(predicate::str::contains("\"kind\": \"remote\""))
        .stdout(predicate::str::contains("\"commit_short\""));

    batch_git(&workspace.path().join("service-one"))
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "service-one  clean  -        up-to-date  main",
        ))
        .stdout(predicate::str::contains(
            "main\n\nsummary: 1 repositories, 1 clean; 0 changed paths",
        ));

    let repository = workspace.path().join("service-one");
    git(&repository, ["config", "user.name", "Batch Git Tests"]);
    git(
        &repository,
        ["config", "user.email", "batch-git@example.invalid"],
    );
    fs::write(repository.join("LOCAL.md"), "local commit\n").unwrap();
    git(&repository, ["add", "LOCAL.md"]);
    git(&repository, ["commit", "-m", "local"]);
    fs::write(repository.join("README.md"), "modified\n").unwrap();
    fs::write(repository.join("UNTRACKED.md"), "untracked\n").unwrap();

    batch_git(&repository)
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "service-one  dirty  M1 ?1    ahead 1   main",
        ))
        .stdout(predicate::str::contains(
            "main\n\nsummary: 1 repositories, 1 dirty; 2 changed paths",
        ))
        .stdout(predicate::str::contains("UNTRACKED.md").not());

    batch_git(&workspace.path().join("service-one"))
        .args(["--", "rev-parse", "--abbrev-ref", "HEAD"])
        .assert()
        .success()
        .stdout(predicate::str::contains("main"));
}

#[test]
fn managed_clone_restore_fetch_and_checkout_form_a_closed_loop() {
    let fixture = Fixture::new("service-two");
    let source = tempfile::tempdir().unwrap();

    batch_git(source.path())
        .args([
            "clone",
            fixture.remote.to_str().unwrap(),
            "nested/service-two",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("registered service-two"));

    let manifest = source.path().join("workspace.toml");
    let target = tempfile::tempdir().unwrap();
    fs::copy(&manifest, target.path().join("workspace.toml")).unwrap();

    batch_git(target.path())
        .args(["--jobs=2", "restore"])
        .assert()
        .success()
        .stdout(predicate::str::contains("service-two  ok      -"));
    assert_eq!(
        git_output(
            &target.path().join("nested/service-two"),
            ["branch", "--show-current"]
        ),
        "main"
    );
    assert_eq!(
        git_output(
            &target.path().join("nested/service-two"),
            ["branch", "--list", "feature"]
        ),
        ""
    );
    assert_eq!(
        git_output(
            &target.path().join("nested/service-two"),
            ["branch", "--remotes", "--list", "origin/feature"]
        ),
        ""
    );
    git(
        &target.path().join("nested/service-two"),
        [
            "config",
            "remote.origin.fetch",
            "+refs/heads/main:refs/remotes/origin/main",
        ],
    );

    batch_git(target.path())
        .args(["fetch"])
        .assert()
        .success()
        .stdout(predicate::str::contains("service-two  ok      -"));
    assert_eq!(
        git_output(
            &target.path().join("nested/service-two"),
            ["branch", "--show-current"]
        ),
        "main"
    );
    assert_eq!(
        git_output(
            &target.path().join("nested/service-two"),
            ["branch", "--remotes", "--list", "origin/feature"]
        ),
        "origin/feature"
    );
    batch_git(target.path())
        .args(["checkout", "feature"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "service-two  ok      feature         -\n\nsummary:",
        ))
        .stdout(predicate::str::contains(
            "summary: 1 ok, 0 skipped, 0 failed",
        ));
    assert_eq!(
        git_output(
            &target.path().join("nested/service-two"),
            ["branch", "--show-current"]
        ),
        "feature"
    );
    assert_eq!(
        git_output(
            &target.path().join("nested/service-two"),
            ["rev-parse", "--abbrev-ref", "@{upstream}"]
        ),
        "origin/feature"
    );
    batch_git(target.path())
        .args(["cc", "-d"])
        .assert()
        .success()
        .stdout(predicate::str::contains("service-two  ok      main"));
    assert_eq!(
        git_output(
            &target.path().join("nested/service-two"),
            ["branch", "--show-current"]
        ),
        "main"
    );
    batch_git(target.path())
        .env("CURRENT_FEATURE_BRANCH", "feature")
        .args(["cf"])
        .assert()
        .success()
        .stdout(predicate::str::contains("service-two  ok      feature"));
    assert_eq!(
        git_output(
            &target.path().join("nested/service-two"),
            ["branch", "--show-current"]
        ),
        "feature"
    );
    batch_git(target.path()).args(["cd"]).assert().success();
    batch_git(target.path())
        .args(["checkout", "--default", "main"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "the argument '--default' cannot be used with '[BRANCH]'",
        ));
    batch_git(target.path())
        .args(["cc", "feature"])
        .assert()
        .success();
    batch_git(target.path())
        .args(["branch"])
        .assert()
        .success()
        .stdout(predicate::str::contains("service-two  feature"));

    batch_git(target.path())
        .args(["checkout", "does-not-exist"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "service-two  skipped  feature         -\n\nsummary:",
        ))
        .stdout(predicate::str::contains(
            "summary: 0 ok, 1 skipped, 0 failed",
        ));

    batch_git(target.path())
        .args(["checkout", "-b", "work", "--from", "main"])
        .assert()
        .success()
        .stdout(predicate::str::contains("service-two  ok      work"))
        .stdout(predicate::str::contains(
            "summary: 1 ok, 0 skipped, 0 failed",
        ));
    assert_eq!(
        git_output(
            &target.path().join("nested/service-two"),
            ["branch", "--show-current"]
        ),
        "work"
    );

    batch_git(target.path())
        .args(["checkout", "-b", "work"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("service-two  failed  work"))
        .stdout(predicate::str::contains("branch already exists: work"));

    batch_git(target.path())
        .args([
            "checkout",
            "-b",
            "tracked-feature",
            "--from",
            "feature",
            "--remote",
            "origin",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "service-two  ok      tracked-feature",
        ));
    assert_eq!(
        git_output(
            &target.path().join("nested/service-two"),
            ["rev-parse", "--abbrev-ref", "@{upstream}"]
        ),
        "origin/feature"
    );

    batch_git(target.path())
        .args(["restore"])
        .assert()
        .success()
        .stdout(predicate::str::contains("service-two  skipped  -"));
}

#[test]
fn checkout_feature_without_an_environment_value_is_a_no_op() {
    let directory = tempfile::tempdir().unwrap();

    for option in ["--feature", "--feat", "-f"] {
        batch_git(directory.path())
            .env_remove("CURRENT_FEATURE_BRANCH")
            .args(["checkout", option])
            .assert()
            .success()
            .stdout(predicate::str::contains(
                "CURRENT_FEATURE_BRANCH is not set; nothing to checkout",
            ));
    }

    batch_git(directory.path())
        .env("CURRENT_FEATURE_BRANCH", "   ")
        .args(["cf"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "CURRENT_FEATURE_BRANCH is not set; nothing to checkout",
        ));
}

#[test]
fn forget_only_changes_the_manifest_and_unknown_commands_are_not_forwarded() {
    let fixture = Fixture::new("service-three");
    let workspace = tempfile::tempdir().unwrap();
    batch_git(workspace.path())
        .args(["clone", fixture.remote.to_str().unwrap(), "service-three"])
        .assert()
        .success();

    batch_git(workspace.path())
        .args(["forget", "service-three"])
        .assert()
        .success()
        .stdout(predicate::str::contains("was not deleted"));
    assert!(workspace.path().join("service-three/.git").exists());
    let manifest = fs::read_to_string(workspace.path().join("workspace.toml")).unwrap();
    assert!(!manifest.contains("service-three"));

    batch_git(workspace.path())
        .args(["unknown-command"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unrecognized subcommand"));

    batch_git(workspace.path())
        .args(["load"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unrecognized subcommand"));

    batch_git(workspace.path())
        .args(["update"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unrecognized subcommand"));
}

#[test]
fn merge_alias_optionally_updates_the_current_branch_first() {
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
        .env("BATCH_GIT_MERGE_UPDATE_CURRENT", "true")
        .args(["merge", "feature"])
        .assert()
        .success()
        .stdout(predicate::str::contains("service-merge  ok"));
    assert!(repository.join("REMOTE.md").is_file());
    assert!(repository.join("FEATURE.md").is_file());
}

#[test]
fn merge_cli_setting_overrides_and_validates_the_environment() {
    let workspace = tempfile::tempdir().unwrap();
    batch_git(workspace.path())
        .args(["scan"])
        .assert()
        .success();
    batch_git(workspace.path())
        .env("BATCH_GIT_MERGE_UPDATE_CURRENT", "invalid")
        .args(["merge", "feature"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "invalid BATCH_GIT_MERGE_UPDATE_CURRENT value",
        ));

    batch_git(workspace.path())
        .env("BATCH_GIT_MERGE_UPDATE_CURRENT", "invalid")
        .args(["merge", "--no-update-current", "feature"])
        .assert()
        .success();
}

#[test]
fn invalid_environment_values_fail_instead_of_falling_back() {
    let workspace = tempfile::tempdir().unwrap();
    batch_git(workspace.path())
        .env("BATCH_GIT_JOBS", "0")
        .args(["scan"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("jobs must be at least 1"));

    batch_git(workspace.path())
        .env("BATCH_GIT_SCAN_DEPTH", "0")
        .args(["scan"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("scan depth must be at least 1"));
}

#[test]
fn status_reports_unavailable_repositories_without_file_details() {
    let fixture = Fixture::new("service-unavailable");
    let workspace = tempfile::tempdir().unwrap();
    batch_git(workspace.path())
        .args([
            "clone",
            fixture.remote.to_str().unwrap(),
            "service-unavailable",
        ])
        .assert()
        .success();

    let repository = workspace.path().join("service-unavailable");
    fs::rename(&repository, workspace.path().join("repository-backup")).unwrap();
    batch_git(workspace.path())
        .args(["status"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "service-unavailable  missing  -        -         -",
        ))
        .stdout(predicate::str::contains(
            "-\n\nsummary: 1 repositories, 1 missing; 0 changed paths",
        ));
    batch_git(workspace.path())
        .args(["info", "service-unavailable"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("STATE           missing"))
        .stdout(predicate::str::contains("CURRENT BRANCH  -"));
    batch_git(workspace.path())
        .args(["info"])
        .assert()
        .success()
        .stdout(predicate::str::contains("MISSING       1"));

    fs::create_dir(&repository).unwrap();
    batch_git(workspace.path())
        .args(["status"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "service-unavailable  not-git  -        -         -",
        ))
        .stdout(predicate::str::contains(
            "-\n\nsummary: 1 repositories, 1 not-git; 0 changed paths",
        ));
}

#[test]
fn parallel_passthrough_covers_all_repositories_in_stable_order() {
    let first = Fixture::new("alpha");
    let second = Fixture::new("beta");
    let workspace = tempfile::tempdir().unwrap();
    git(
        workspace.path(),
        ["clone", first.remote.to_str().unwrap(), "alpha"],
    );
    git(
        workspace.path(),
        ["clone", second.remote.to_str().unwrap(), "beta"],
    );
    batch_git(workspace.path())
        .args(["--jobs=2", "scan"])
        .assert()
        .success();

    let output = batch_git(workspace.path())
        .args(["--jobs=2", "--", "branch", "--show-current"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8(output).unwrap();
    let footer = "-".repeat(72);
    let alpha_begin = output.find("-- alpha [ok] ").unwrap();
    let alpha_end = output[alpha_begin..].find(footer.as_str()).unwrap() + alpha_begin;
    let beta_begin = output.find("-- beta [ok] ").unwrap();
    let beta_end = output[beta_begin..].find(footer.as_str()).unwrap() + beta_begin;
    assert!(
        alpha_begin < alpha_end && alpha_end < beta_begin && beta_begin < beta_end,
        "each repository must have a distinct output block in stable manifest order"
    );
    assert_eq!(output.matches("\nmain\n").count(), 2);
    assert_eq!(output.matches(footer.as_str()).count(), 2);

    batch_git(workspace.path())
        .args(["--", "status", "--short"])
        .assert()
        .success()
        .stdout(predicate::str::contains("alpha  ok\nbeta   ok"))
        .stdout(predicate::str::contains("-- alpha").not());

    batch_git(workspace.path())
        .env("BATCH_GIT_PASSTHROUGH_VERBOSE", "false")
        .args(["--", "branch", "--show-current"])
        .assert()
        .success()
        .stdout(predicate::str::contains("alpha  ok\nbeta   ok"))
        .stdout(predicate::str::contains("output hidden").not());

    batch_git(workspace.path())
        .args(["--", "commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("alpha [skipped]"))
        .stdout(predicate::str::contains("beta [skipped]"))
        .stdout(predicate::str::contains(
            "summary: 0 ok, 2 skipped, 0 failed",
        ));

    batch_git(workspace.path())
        .args(["exec", "alpha", "--", "commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("alpha       skipped"))
        .stdout(predicate::str::contains(
            "summary: 0 ok, 1 skipped, 0 failed",
        ));

    batch_git(workspace.path())
        .args(["--", "config", "--get", "batch-git.missing"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("alpha  failed - exit 1"))
        .stdout(predicate::str::contains("beta   failed - exit 1"));

    batch_git(workspace.path())
        .args([
            "--verbose",
            "exec",
            "alpha",
            "--",
            "branch",
            "--show-current",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("alpha       ok"))
        .stdout(predicate::str::contains("-- alpha [ok] "))
        .stdout(predicate::str::contains("alpha (alpha)").not())
        .stdout(predicate::str::contains("\nmain\n"))
        .stdout(predicate::str::contains("beta").not());

    batch_git(workspace.path())
        .args(["exec", "--match", "a*", "--", "status", "--short"])
        .assert()
        .success()
        .stdout(predicate::str::contains("alpha       ok"))
        .stdout(predicate::str::contains("beta").not());

    batch_git(workspace.path())
        .args(["exec", "missing", "--", "status"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "unknown repository selector: missing",
        ));
}

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
fn schedule_plans_runs_and_generates_native_definitions() {
    let fixture = Fixture::new("scheduled-service");
    let workspace = tempfile::tempdir().unwrap();
    batch_git(workspace.path())
        .args([
            "clone",
            fixture.remote.to_str().unwrap(),
            "scheduled-service",
        ])
        .assert()
        .success();

    let manifest_path = workspace.path().join("workspace.toml");
    let mut manifest = fs::read_to_string(&manifest_path).unwrap();
    manifest.push_str(
        r#"

[[schedules]]
name = "nightly-sync"
action = "sync"
at = "02:30"
timezone = "local"
overlap = "skip"

[schedules.scope]
repositories = ["scheduled-service"]

[[schedules]]
name = "business-hours-sync"
action = "pull"
cron = "0 */15 9-17 ? * MON-FRI"
timezone = "local"
overlap = "skip"

[schedules.scope]
repositories = ["scheduled-service"]
"#,
    );
    fs::write(&manifest_path, manifest).unwrap();

    batch_git(workspace.path())
        .args(["schedule", "plan", "nightly-sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("scheduled-service  sync"))
        .stdout(predicate::str::contains("trigger: daily 02:30"));

    batch_git(workspace.path())
        .args(["schedule", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\": \"nightly-sync\""))
        .stdout(predicate::str::contains("\"scope\": \"scheduled-service\""));

    batch_git(workspace.path())
        .args(["schedule", "plan", "business-hours-sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("scheduled-service  pull"))
        .stdout(predicate::str::contains(
            "trigger: cron 0 */15 9-17 ? * MON-FRI",
        ));

    batch_git(workspace.path())
        .args(["schedule", "run", "nightly-sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("scheduled-service  ok"));

    let updater = workspace.path().join("updater");
    git(
        workspace.path(),
        ["clone", fixture.remote.to_str().unwrap(), "updater"],
    );
    git(&updater, ["config", "user.name", "Batch Git Tests"]);
    git(
        &updater,
        ["config", "user.email", "batch-git@example.invalid"],
    );
    fs::write(updater.join("SCHEDULED_PULL.md"), "pulled\n").unwrap();
    git(&updater, ["add", "SCHEDULED_PULL.md"]);
    git(&updater, ["commit", "-m", "scheduled pull"]);
    git(&updater, ["push", "origin", "main"]);

    batch_git(workspace.path())
        .args(["schedule", "run", "business-hours-sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("scheduled-service  ok"));
    assert!(
        workspace
            .path()
            .join("scheduled-service/SCHEDULED_PULL.md")
            .is_file()
    );

    fs::write(
        workspace.path().join("scheduled-service/README.md"),
        "dirty\n",
    )
    .unwrap();
    batch_git(workspace.path())
        .args(["pull", "scheduled-service"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("working tree is not clean"));

    batch_git(workspace.path())
        .args([
            "schedule",
            "generate",
            "nightly-sync",
            "--platform",
            "launchd",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("<key>ProgramArguments</key>"))
        .stdout(predicate::str::contains("<string>schedule</string>"))
        .stdout(predicate::str::contains("<string>run</string>"))
        .stdout(predicate::str::contains("<string>nightly-sync</string>"))
        .stdout(predicate::str::contains("BATCH_GIT_WORKSPACE"))
        .stdout(predicate::str::contains(
            "<key>StandardOutPath</key><string>/dev/null</string>",
        ))
        .stdout(predicate::str::contains(
            "<key>StandardErrorPath</key><string>/dev/null</string>",
        ));

    batch_git(workspace.path())
        .args([
            "schedule",
            "generate",
            "business-hours-sync",
            "--platform",
            "systemd",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "OnCalendar=Mon,Tue,Wed,Thu,Fri *-*-* 9,10,11,12,13,14,15,16,17:0,15,30,45:0",
        ));

    batch_git(workspace.path())
        .env("BATCH_GIT_SCHEDULE_LOG", "false")
        .args([
            "schedule",
            "generate",
            "nightly-sync",
            "--platform",
            "launchd",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "<key>StandardOutPath</key><string>/dev/null</string>",
        ))
        .stdout(predicate::str::contains(
            "<key>StandardErrorPath</key><string>/dev/null</string>",
        ));

    batch_git(workspace.path())
        .env("BATCH_GIT_SCHEDULE_LOG", "false")
        .args([
            "schedule",
            "generate",
            "nightly-sync",
            "--platform",
            "systemd",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("StandardOutput=null"))
        .stdout(predicate::str::contains("StandardError=null"));

    batch_git(workspace.path())
        .env("BATCH_GIT_SCHEDULE_LOG", "invalid")
        .args([
            "schedule",
            "generate",
            "nightly-sync",
            "--platform",
            "launchd",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "invalid BATCH_GIT_SCHEDULE_LOG value",
        ));

    batch_git(workspace.path())
        .env("BATCH_GIT_STATE_DIR", workspace.path().join("state"))
        .args([
            "schedule",
            "register",
            "nightly-sync",
            "--platform",
            "launchd",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "would register schedule nightly-sync on launchd",
        ));

    batch_git(workspace.path())
        .env("BATCH_GIT_STATE_DIR", workspace.path().join("state"))
        .args(["schedule", "unregister", "nightly-sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "schedule nightly-sync already absent",
        ));
}

#[cfg(unix)]
#[test]
fn schedule_register_updates_same_name_and_unregisters() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new("registered-service");
    let workspace = tempfile::tempdir().unwrap();
    batch_git(workspace.path())
        .args([
            "clone",
            fixture.remote.to_str().unwrap(),
            "registered-service",
        ])
        .assert()
        .success();

    let manifest_path = workspace.path().join("workspace.toml");
    batch_git(workspace.path())
        .args([
            "schedule",
            "add",
            "registered-sync",
            "--action",
            "pull",
            "--cron",
            "0 30 2 * * *",
            "--all",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("schedule registered-sync added"));

    let home = workspace.path().join("home");
    let state = workspace.path().join("state");
    let bin = workspace.path().join("fake-bin");
    let calls = workspace.path().join("scheduler-calls.log");
    fs::create_dir_all(&bin).unwrap();
    let launchctl = bin.join("launchctl");
    fs::write(
        &launchctl,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$BATCH_GIT_TEST_CALLS\"\n",
    )
    .unwrap();
    fs::set_permissions(&launchctl, fs::Permissions::from_mode(0o755)).unwrap();
    let id = bin.join("id");
    fs::write(&id, "#!/bin/sh\nprintf '501\\n'\n").unwrap();
    fs::set_permissions(&id, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let configure = |command: &mut assert_cmd::Command| {
        command
            .env("HOME", &home)
            .env("PATH", &path)
            .env("BATCH_GIT_STATE_DIR", &state)
            .env("BATCH_GIT_SCHEDULE_LOG", "true")
            .env("BATCH_GIT_TEST_CALLS", &calls);
    };

    let mut first = batch_git(workspace.path());
    configure(&mut first);
    first
        .args([
            "schedule",
            "register",
            "registered-sync",
            "--platform",
            "launchd",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "schedule registered-sync registered on launchd",
        ));

    let mut status = batch_git(workspace.path());
    configure(&mut status);
    status
        .args(["schedule", "status", "registered-sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("NATIVE LOADED"))
        .stdout(predicate::str::contains("DEFINITION MATCHES"))
        .stdout(predicate::str::contains("LOGGING"))
        .stdout(predicate::str::contains("stdout.log"))
        .stdout(predicate::str::contains("stderr.log"));

    let agents = home.join("Library/LaunchAgents");
    let plist = fs::read_dir(&agents)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(
        fs::read_to_string(&plist)
            .unwrap()
            .contains("<integer>2</integer>")
    );

    let mut unchanged = batch_git(workspace.path());
    configure(&mut unchanged);
    unchanged
        .args([
            "schedule",
            "register",
            "registered-sync",
            "--platform",
            "launchd",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "schedule registered-sync unchanged",
        ));

    let mut edit = batch_git(workspace.path());
    configure(&mut edit);
    edit.args(["schedule", "update", "registered-sync", "--at", "03:45"])
        .assert()
        .success()
        .stdout(predicate::str::contains("schedule registered-sync updated"))
        .stdout(predicate::str::contains(
            "schedule register registered-sync",
        ));
    let mut update = batch_git(workspace.path());
    configure(&mut update);
    update
        .args([
            "schedule",
            "register",
            "registered-sync",
            "--platform",
            "launchd",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "schedule registered-sync updated on launchd",
        ));
    let updated = fs::read_to_string(&plist).unwrap();
    assert!(updated.contains("<integer>3</integer>"));
    assert!(updated.contains("<integer>45</integer>"));
    assert_eq!(fs::read_dir(&agents).unwrap().count(), 1);

    let mut disable_logs = batch_git(workspace.path());
    configure(&mut disable_logs);
    disable_logs
        .env("BATCH_GIT_SCHEDULE_LOG", "false")
        .args([
            "schedule",
            "register",
            "registered-sync",
            "--platform",
            "launchd",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "schedule registered-sync updated on launchd",
        ));
    let disabled = fs::read_to_string(&plist).unwrap();
    assert!(disabled.contains("<key>StandardOutPath</key><string>/dev/null</string>"));
    assert!(disabled.contains("<key>StandardErrorPath</key><string>/dev/null</string>"));

    let mut disabled_status = batch_git(workspace.path());
    configure(&mut disabled_status);
    disabled_status
        .args(["schedule", "status", "registered-sync", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"logging\": false"))
        .stdout(predicate::str::contains("\"stdout\": null"))
        .stdout(predicate::str::contains("\"stderr\": null"));

    let mut protected_remove = batch_git(workspace.path());
    configure(&mut protected_remove);
    protected_remove
        .args(["schedule", "remove", "registered-sync"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "is registered; run schedule unregister first",
        ));

    let mut remove = batch_git(workspace.path());
    configure(&mut remove);
    remove
        .args(["schedule", "remove", "registered-sync", "--unregister"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "schedule registered-sync unregistered from launchd",
        ))
        .stdout(predicate::str::contains("schedule registered-sync removed"));
    assert!(!plist.exists());
    assert!(
        !fs::read_to_string(&manifest_path)
            .unwrap()
            .contains("registered-sync")
    );
    let calls = fs::read_to_string(calls).unwrap();
    assert!(calls.contains("bootstrap gui/501"));
    assert!(calls.contains("bootout gui/501/com.batch-git."));
}

fn batch_git(directory: &Path) -> assert_cmd::Command {
    let mut command =
        assert_cmd::Command::from_std(Command::cargo_bin("batch-git").expect("batch-git binary"));
    command.current_dir(directory);
    command
}

fn git<const N: usize>(directory: &Path, args: [&str; N]) {
    let status = Command::new("git")
        .current_dir(directory)
        .args(args)
        .status()
        .expect("run git");
    assert!(status.success());
}

fn git_output<const N: usize>(directory: &Path, args: [&str; N]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .args(args)
        .output()
        .expect("run git");
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
