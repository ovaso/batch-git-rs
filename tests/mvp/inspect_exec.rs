use super::*;

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
fn env_ls_shows_supported_variables_defaults_and_effective_values() {
    let workspace = tempfile::tempdir().unwrap();
    let state_directory = workspace.path().join("state");
    let output = batch_git(workspace.path())
        .env_remove("BATCH_GIT_WORKSPACE")
        .env("BATCH_GIT_JOBS", "6")
        .env("BATCH_GIT_SCAN_DEPTH", "3")
        .env("BATCH_GIT_STATE_DIR", &state_directory)
        .env("BATCH_GIT_SCHEDULE_LOG", "yes")
        .env("BATCH_GIT_TZ", "Asia/Shanghai")
        .env("BATCH_GIT_REMOTE", "origin")
        .env("CURRENT_FEATURE_BRANCH", "feature/config")
        .env("BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE", "off")
        .env("BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT", "on")
        .env("BATCH_GIT_PASSTHROUGH_VERBOSE", "false")
        .env("NO_COLOR", "1")
        .args(["--jobs", "8", "env", "ls"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("VARIABLE"));
    assert!(!stdout.contains("DESCRIPTION"));
    assert!(stdout.contains("DEFAULT"));
    assert!(stdout.contains("CURRENT"));
    assert!(
        !stdout.contains('\u{1b}'),
        "non-TTY output must stay plain text"
    );

    let jobs = stdout
        .lines()
        .find(|line| line.starts_with("BATCH_GIT_JOBS"))
        .expect("jobs row");
    assert!(!jobs.contains("Maximum number of repositories operated concurrently."));
    assert!(
        jobs.ends_with("8"),
        "CLI --jobs must be the effective value"
    );

    let scan_depth = stdout
        .lines()
        .find(|line| line.starts_with("BATCH_GIT_SCAN_DEPTH"))
        .expect("scan-depth row");
    assert!(scan_depth.ends_with("3"));
    let schedule_log = stdout
        .lines()
        .find(|line| line.starts_with("BATCH_GIT_SCHEDULE_LOG"))
        .expect("schedule-log row");
    assert!(schedule_log.ends_with("true"));
    let state_directory_row = stdout
        .lines()
        .find(|line| line.starts_with("BATCH_GIT_STATE_DIR"))
        .expect("state-directory row");
    assert!(state_directory_row.ends_with(&state_directory.to_string_lossy().to_string()));
    assert!(stdout.contains("BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE"));
    assert!(stdout.contains("CURRENT_FEATURE_BRANCH"));
    assert!(stdout.contains("NO_COLOR"));

    batch_git(workspace.path())
        .env("BATCH_GIT_STATE_DIR", &state_directory)
        .env("NO_COLOR", "1")
        .args(["env", "ls", "-d"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DESCRIPTION"))
        .stdout(predicate::str::contains(
            "Maximum number of repositories operated concurrently.",
        ));
}

#[test]
fn env_ls_rejects_invalid_effective_values() {
    let workspace = tempfile::tempdir().unwrap();
    batch_git(workspace.path())
        .env("BATCH_GIT_STATE_DIR", workspace.path().join("state"))
        .env("BATCH_GIT_SCAN_DEPTH", "0")
        .args(["env", "ls"])
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
