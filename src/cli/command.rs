//! Clap definitions for the stable top-level command surface.

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::automation::{self, OutputFormat};

use super::args::*;
use super::invocation::RuntimeOptions;

/// clap 解析后的顶层命令行结构。
#[derive(Debug, Parser)]
#[command(
    name = "batch-git",
    version,
    about = "Operate a workspace containing multiple Git repositories",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Maximum number of repositories operated concurrently.
    #[arg(long, global = true)]
    pub jobs: Option<usize>,

    /// Show successful child command output.
    #[arg(long, global = true)]
    pub verbose: bool,

    /// Render the versioned automation protocol as text, JSON, or JSON Lines.
    #[arg(long, global = true, value_enum)]
    pub output: Option<OutputFormat>,

    /// Opaque identifier echoed by machine-readable receipts and events.
    #[arg(long, global = true)]
    pub request_id: Option<String>,

    /// Disable Git prompts and interactive child-process input.
    #[arg(long, global = true)]
    pub non_interactive: bool,

    /// Limit each child Git process, using a duration such as 30s, 5m, or 1h.
    #[arg(long, global = true)]
    pub timeout: Option<String>,

    /// Resolve a no-side-effect plan instead of executing the command.
    #[arg(long, global = true, conflicts_with = "apply")]
    pub plan: bool,

    /// Execute only when --expect-workspace-revision still matches the planned workspace.
    #[arg(
        long,
        global = true,
        requires = "expect_workspace_revision",
        conflicts_with = "plan"
    )]
    pub apply: bool,

    /// Manifest digest returned by a previous --plan response.
    #[arg(long, global = true, requires = "apply")]
    pub expect_workspace_revision: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

/// 所有受 batch-git 约束和解释的内建命令。
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Stage all non-ignored additions, modifications, and deletions in selected repositories.
    Add(SyncArgs),
    /// Show the current branch of every registered repository.
    #[command(visible_alias = "b")]
    Branch(MachineReadableArgs),
    /// Show supported automation protocol features and command names.
    Capabilities,
    /// Checkout each repository's declared default branch.
    Cd,
    /// Checkout the branch named by CURRENT_FEATURE_BRANCH.
    Cf,
    /// Checkout a branch wherever it exists.
    #[command(visible_alias = "cc")]
    Checkout(CheckoutArgs),
    /// Clone one repository and register it in workspace.toml.
    Clone(CloneArgs),
    /// Commit already-staged changes without staging additional content.
    Commit(CommitArgs),
    /// Inspect supported environment variables and their effective values.
    Env(EnvArgs),
    /// Run a Git command in selected repositories.
    Exec(ExecArgs),
    /// Fetch all remotes with pruning, without merging.
    Fetch,
    /// Find local and synchronized remote branches.
    #[command(visible_aliases = ["fd", "f"])]
    Find(FindArgs),
    /// Remove registrations without deleting repository directories.
    Forget(ForgetArgs),
    /// Show workspace or repository metadata.
    #[command(visible_alias = "i")]
    Info(InfoArgs),
    /// List registered repositories.
    #[command(visible_aliases = ["ls", "l"])]
    List(ListArgs),
    /// Merge a source branch into each repository's current branch.
    #[command(visible_alias = "m")]
    Merge(MergeArgs),
    /// Fast-forward the current tracking branch in selected repositories.
    Pull(SyncArgs),
    /// Push the current tracking branch in selected repositories.
    Push(PushArgs),
    /// Restore missing repositories declared in workspace.toml.
    Restore,
    /// Scan for repositories and create or extend workspace.toml.
    Scan(ScanArgs),
    /// Manage scheduled workspace synchronization.
    #[cfg(feature = "schedule")]
    Schedule(ScheduleArgs),
    /// Print a versioned JSON Schema for an automation document.
    Schema(SchemaArgs),
    /// Show a compact status summary for every registered repository.
    #[command(visible_alias = "s")]
    Status(MachineReadableArgs),
    /// Restore missing repositories and fetch remote refs without changing worktrees.
    Sync(SyncArgs),
    /// Remove all staged changes from selected indexes without changing working-tree files.
    Unstage(SyncArgs),
}

impl Cli {
    /// Resolve global execution options after clap has validated the syntax.
    pub fn runtime_options(&self) -> Result<RuntimeOptions> {
        let options = RuntimeOptions {
            jobs: self.jobs,
            verbose: self.verbose,
            output: self.output.unwrap_or(OutputFormat::Text),
            request_id: self.request_id.clone(),
            non_interactive: self.non_interactive,
            timeout: self
                .timeout
                .as_deref()
                .map(automation::parse_timeout)
                .transpose()?,
            plan: self.plan,
            apply: self.apply,
            expected_workspace_revision: self.expect_workspace_revision.clone(),
        };
        options.automation().validate()?;
        Ok(options)
    }
}

impl Command {
    /// Identify commands that can write the manifest, repositories, remotes, or scheduler state.
    pub fn is_mutating(&self) -> bool {
        match self {
            Self::Branch(_)
            | Self::Capabilities
            | Self::Env(_)
            | Self::Find(_)
            | Self::Info(_)
            | Self::List(_)
            | Self::Schema(_)
            | Self::Status(_) => false,
            #[cfg(feature = "schedule")]
            Self::Schedule(arguments) => arguments.command.is_mutating(),
            Self::Add(_)
            | Self::Clone(_)
            | Self::Commit(_)
            | Self::Cd
            | Self::Cf
            | Self::Checkout(_)
            | Self::Exec(_)
            | Self::Fetch
            | Self::Forget(_)
            | Self::Merge(_)
            | Self::Pull(_)
            | Self::Push(_)
            | Self::Restore
            | Self::Scan(_)
            | Self::Sync(_)
            | Self::Unstage(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn help_commands_are_alphabetically_sorted() {
        let command = Cli::command();
        assert_sorted(command.get_subcommands().map(|command| command.get_name()));

        #[cfg(feature = "schedule")]
        {
            let schedule = command
                .get_subcommands()
                .find(|command| command.get_name() == "schedule")
                .expect("schedule subcommand");
            assert_sorted(schedule.get_subcommands().map(|command| command.get_name()));
        }
    }

    #[test]
    fn top_level_command_surface_remains_flat_and_keeps_shortcuts() {
        let command = Cli::command();
        let names = command
            .get_subcommands()
            .map(|command| command.get_name())
            .collect::<Vec<_>>();
        let expected = vec![
            "add",
            "branch",
            "capabilities",
            "cd",
            "cf",
            "checkout",
            "clone",
            "commit",
            "env",
            "exec",
            "fetch",
            "find",
            "forget",
            "info",
            "list",
            "merge",
            "pull",
            "push",
            "restore",
            "scan",
            "schema",
            "status",
            "sync",
            "unstage",
        ];
        #[cfg(feature = "schedule")]
        let expected = {
            let mut expected = expected;
            expected.insert(20, "schedule");
            expected
        };
        assert_eq!(
            names, expected,
            "command organization changes must not introduce nested command paths"
        );
    }

    fn assert_sorted<'a>(names: impl Iterator<Item = &'a str>) {
        let names: Vec<_> = names.collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }
}
