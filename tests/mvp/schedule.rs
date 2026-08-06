use super::*;

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
        .env("BATCH_GIT_STATE_DIR", workspace.path().join("state"))
        .args([
            "schedule",
            "generate",
            "nightly-sync",
            "--platform",
            "windows",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("<CalendarTrigger>"))
        .stdout(predicate::str::contains(
            "<StartBoundary>2000-01-01T02:30:00</StartBoundary>",
        ))
        .stdout(predicate::str::contains(
            "<Arguments>schedule run nightly-sync</Arguments>",
        ));

    batch_git(workspace.path())
        .env("BATCH_GIT_STATE_DIR", workspace.path().join("state"))
        .args([
            "schedule",
            "generate",
            "business-hours-sync",
            "--platform",
            "windows",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "Windows Task Scheduler does not support cron schedules",
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
        .env("BATCH_GIT_TZ", "Asia/Shanghai")
        .args([
            "schedule",
            "generate",
            "nightly-sync",
            "--platform",
            "systemd",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Environment=\"BATCH_GIT_TZ=Asia/Shanghai\"",
        ))
        .stdout(predicate::str::contains(
            "OnCalendar=*-*-* 02:30:00 Asia/Shanghai",
        ))
        .stdout(predicate::str::contains("StandardOutput=null"))
        .stdout(predicate::str::contains("StandardError=null"));

    batch_git(workspace.path())
        .env("BATCH_GIT_TZ", "Asia/Shanghai")
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
            "<key>BATCH_GIT_TZ</key><string>Asia/Shanghai</string>",
        ));

    batch_git(workspace.path())
        .env("BATCH_GIT_STATE_DIR", workspace.path().join("state"))
        .env("BATCH_GIT_TZ", "Asia/Shanghai")
        .args([
            "schedule",
            "generate",
            "nightly-sync",
            "--platform",
            "windows",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "<Arguments>schedule native-run nightly-sync --timezone Asia/Shanghai</Arguments>",
        ));

    batch_git(workspace.path())
        .env("BATCH_GIT_TZ", "Asia / Shanghai")
        .args([
            "schedule",
            "generate",
            "nightly-sync",
            "--platform",
            "systemd",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "BATCH_GIT_TZ must not contain whitespace",
        ));

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
