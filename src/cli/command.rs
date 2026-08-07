//! Clap definitions for the stable top-level command surface.

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::automation::{self, OutputFormat};

use super::args::*;
use super::invocation::RuntimeOptions;
use super::metadata::CommandKind;

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
    /// Clone one repository and register it in batchspace.toml.
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
    /// Restore missing repositories declared in batchspace.toml.
    Restore,
    /// Scan for repositories and create or extend batchspace.toml.
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
    /// Return the payload-independent command family used by compile-time metadata.
    pub(crate) fn kind(&self) -> CommandKind {
        match self {
            Self::Add(_) => CommandKind::Add,
            Self::Branch(_) => CommandKind::Branch,
            Self::Capabilities => CommandKind::Capabilities,
            Self::Cd => CommandKind::Cd,
            Self::Cf => CommandKind::Cf,
            Self::Checkout(_) => CommandKind::Checkout,
            Self::Clone(_) => CommandKind::Clone,
            Self::Commit(_) => CommandKind::Commit,
            Self::Env(_) => CommandKind::Env,
            Self::Exec(_) => CommandKind::Exec,
            Self::Fetch => CommandKind::Fetch,
            Self::Find(_) => CommandKind::Find,
            Self::Forget(_) => CommandKind::Forget,
            Self::Info(_) => CommandKind::Info,
            Self::List(_) => CommandKind::List,
            Self::Merge(_) => CommandKind::Merge,
            Self::Pull(_) => CommandKind::Pull,
            Self::Push(_) => CommandKind::Push,
            Self::Restore => CommandKind::Restore,
            Self::Scan(_) => CommandKind::Scan,
            #[cfg(feature = "schedule")]
            Self::Schedule(_) => CommandKind::Schedule,
            Self::Schema(_) => CommandKind::Schema,
            Self::Status(_) => CommandKind::Status,
            Self::Sync(_) => CommandKind::Sync,
            Self::Unstage(_) => CommandKind::Unstage,
        }
    }

    /// Identify commands that can write the manifest, repositories, remotes, or scheduler state.
    pub fn is_mutating(&self) -> bool {
        #[cfg(feature = "schedule")]
        if let Self::Schedule(arguments) = self {
            return arguments.command.is_mutating();
        }
        self.kind().metadata().mutating
    }

    /// Whether the global plan/apply protocol has a complete handler for this command family.
    pub(crate) fn supports_global_plan(&self) -> bool {
        self.kind().metadata().global_plan
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;
    use crate::cli::command_metadata;

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

    #[test]
    fn clap_surface_and_command_metadata_have_identical_canonical_names() {
        let clap = Cli::command();
        let clap_names = clap
            .get_subcommands()
            .map(|command| command.get_name().to_owned())
            .collect::<Vec<_>>();
        let metadata_names = command_metadata()
            .iter()
            .map(|command| command.name.to_owned())
            .collect::<Vec<_>>();
        assert_eq!(clap_names, metadata_names);
        for (clap_command, metadata) in clap.get_subcommands().zip(command_metadata()) {
            assert_eq!(
                clap_command.get_visible_aliases().collect::<Vec<_>>(),
                metadata.aliases,
                "aliases differ for {}",
                metadata.name
            );
        }
        assert!(
            command_metadata()
                .iter()
                .all(|command| !command.global_plan || command.mutating)
        );
    }

    fn assert_sorted<'a>(names: impl Iterator<Item = &'a str>) {
        let names: Vec<_> = names.collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }
}
