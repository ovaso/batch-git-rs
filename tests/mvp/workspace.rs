use super::*;

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
fn managed_clone_proxies_branch_depth_and_single_branch_to_git() {
    let fixture = Fixture::new("clone-options");
    let workspace = tempfile::tempdir().unwrap();

    batch_git(workspace.path())
        .args([
            "clone",
            fixture.remote.to_str().unwrap(),
            "shallow-main",
            "--branch",
            "main",
            "--depth",
            "1",
            "--single-branch",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("registered shallow-main"));

    let repository = workspace.path().join("shallow-main");
    assert!(repository.join(".git/shallow").is_file());
    assert_eq!(
        git_output(&repository, ["branch", "--show-current"]),
        "main"
    );
    assert_eq!(git_output(&repository, ["branch", "-r"]), "origin/main");
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
