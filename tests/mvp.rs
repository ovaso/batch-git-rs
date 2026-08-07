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

#[path = "mvp/branches.rs"]
mod branches;
#[path = "mvp/changes.rs"]
mod changes;
#[path = "mvp/inspect_exec.rs"]
mod inspect_exec;
#[path = "mvp/remote.rs"]
mod remote;
#[cfg(feature = "schedule")]
#[path = "mvp/schedule.rs"]
mod schedule;
#[path = "mvp/workspace.rs"]
mod workspace;

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

fn git_succeeds<const N: usize>(directory: &Path, args: [&str; N]) -> bool {
    Command::new("git")
        .current_dir(directory)
        .args(args)
        .output()
        .expect("run git")
        .status
        .success()
}

fn git_ref_exists(git_directory: &Path, reference: &str) -> bool {
    Command::new("git")
        .args([
            "--git-dir",
            git_directory.to_str().unwrap(),
            "show-ref",
            "--verify",
            "--quiet",
            reference,
        ])
        .status()
        .expect("run git")
        .success()
}

fn set_default_branch(workspace: &Path, repository_name: &str, default_branch: &str) {
    let manifest_path = workspace.join("batchspace.toml");
    let mut manifest: toml::Value =
        toml::from_str(&fs::read_to_string(&manifest_path).expect("read workspace manifest"))
            .expect("parse workspace manifest");
    let repositories = manifest
        .get_mut("repositories")
        .and_then(toml::Value::as_array_mut)
        .expect("workspace manifest repositories");
    let repository = repositories
        .iter_mut()
        .find(|record| record.get("name").and_then(toml::Value::as_str) == Some(repository_name))
        .expect("registered repository");
    repository["default_branch"] = toml::Value::String(default_branch.to_owned());
    fs::write(
        manifest_path,
        toml::to_string_pretty(&manifest).expect("serialize workspace manifest"),
    )
    .expect("write workspace manifest");
}
