//! Command-line parsing and the explicit `--` passthrough boundary.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};

#[derive(Debug, Clone, Default)]
pub struct RuntimeOptions {
    pub jobs: Option<usize>,
    pub verbose: bool,
}

pub enum Invocation {
    BuiltIn(Vec<OsString>),
    Passthrough {
        args: Vec<OsString>,
        options: RuntimeOptions,
    },
}

pub fn parse_invocation(args: Vec<OsString>) -> Result<Invocation> {
    if args.is_empty() {
        bail!("missing argv[0]");
    }

    let mut index = 1;
    let mut options = RuntimeOptions::default();
    while index < args.len() {
        let value = args[index].to_string_lossy();
        if value == "--verbose" {
            options.verbose = true;
            index += 1;
        } else if value == "--jobs" {
            let Some(raw) = args.get(index + 1) else {
                bail!("--jobs requires a value");
            };
            options.jobs = Some(parse_jobs(raw)?);
            index += 2;
        } else if let Some(raw) = value.strip_prefix("--jobs=") {
            options.jobs = Some(parse_jobs(&OsString::from(raw))?);
            index += 1;
        } else {
            break;
        }
    }

    if args.get(index).is_some_and(|value| value == "--") {
        let passthrough = args[index + 1..].to_vec();
        if passthrough.is_empty() {
            bail!("Git passthrough requires arguments after --");
        }
        return Ok(Invocation::Passthrough {
            args: passthrough,
            options,
        });
    }

    Ok(Invocation::BuiltIn(args))
}

fn parse_jobs(value: &OsString) -> Result<usize> {
    let jobs = value
        .to_string_lossy()
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("invalid --jobs value: {}", value.to_string_lossy()))?;
    if jobs == 0 {
        bail!("--jobs must be at least 1");
    }
    Ok(jobs)
}

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

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show the current branch of every registered repository.
    #[command(visible_alias = "b")]
    Branch,
    /// Checkout each repository's declared default branch.
    Cd,
    /// Checkout the branch named by CURRENT_FEATURE_BRANCH.
    Cf,
    /// Checkout a branch wherever it exists.
    #[command(visible_alias = "cc")]
    Checkout(CheckoutArgs),
    /// Clone one repository and register it in workspace.toml.
    Clone(CloneArgs),
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
    /// Merge a branch into the current branch wherever it exists.
    #[command(visible_alias = "m")]
    Merge(MergeArgs),
    /// Fast-forward the current tracking branch in selected repositories.
    Pull(SyncArgs),
    /// Restore missing repositories declared in workspace.toml.
    Restore,
    /// Scan for repositories and create or extend workspace.toml.
    Scan(ScanArgs),
    /// Manage scheduled workspace synchronization.
    Schedule(ScheduleArgs),
    /// Show a compact status summary for every registered repository.
    #[command(visible_alias = "s")]
    Status,
    /// Restore missing repositories and fetch remote refs without changing worktrees.
    Sync(SyncArgs),
}

#[derive(Debug, Args)]
pub struct CloneArgs {
    /// Repository URL.
    pub repository: String,

    /// Workspace-relative destination directory.
    pub directory: Option<PathBuf>,

    /// Initially checkout this branch.
    #[arg(short = 'b', long)]
    pub branch: Option<String>,

    /// Create a shallow clone with this history depth.
    #[arg(long)]
    pub depth: Option<usize>,

    /// Clone only the selected/default branch.
    #[arg(long)]
    pub single_branch: bool,
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    /// Maximum directory depth to scan; defaults to BATCH_GIT_SCAN_DEPTH or 1.
    #[arg(short = 'd', long)]
    pub depth: Option<usize>,
}

#[derive(Debug, Args)]
pub struct SyncArgs {
    /// Repository name or workspace-relative directory; defaults to the whole workspace.
    pub selectors: Vec<String>,

    /// Select repository names using the same '*' wildcard as find.
    #[arg(long = "match", value_name = "PATTERN")]
    pub matches: Vec<String>,

    /// Explicitly select the whole workspace.
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct ScheduleArgs {
    #[command(subcommand)]
    pub command: ScheduleCommand,
}

#[derive(Debug, Subcommand)]
pub enum ScheduleCommand {
    /// Add a schedule declaration to workspace.toml.
    #[command(visible_alias = "create")]
    Add(ScheduleAddArgs),
    /// Validate a schedule and the selected platform.
    Doctor(ScheduleDoctorArgs),
    /// Generate a native scheduler definition without registering it.
    Generate(SchedulePlatformArgs),
    /// List declared schedules.
    List(ScheduleListArgs),
    /// Preview the repositories and action selected by a schedule.
    Plan(ScheduleNameArgs),
    /// Create or update a native scheduler task.
    #[command(visible_alias = "install")]
    Register(ScheduleRegisterArgs),
    /// Remove a schedule declaration from workspace.toml.
    #[command(visible_alias = "delete")]
    Remove(ScheduleRemoveArgs),
    /// Run a declared schedule immediately.
    Run(ScheduleRunArgs),
    /// Show declaration and local registration state.
    Status(ScheduleNameArgs),
    /// Remove a native scheduler task without deleting its declaration.
    #[command(visible_alias = "uninstall")]
    Unregister(ScheduleUnregisterArgs),
    /// Update an existing schedule declaration.
    #[command(visible_alias = "edit")]
    Update(ScheduleUpdateArgs),
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ScheduleOverlapValue {
    Skip,
    Queue,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ScheduleActionValue {
    Sync,
    Pull,
}

#[derive(Debug, Args)]
#[command(
    group(clap::ArgGroup::new("trigger").required(true).args(["at", "every", "cron"])),
    group(clap::ArgGroup::new("scope").required(true).args(["all", "repositories"]))
)]
pub struct ScheduleAddArgs {
    pub name: String,

    /// Operation executed by the schedule.
    #[arg(long, value_enum, default_value = "sync")]
    pub action: ScheduleActionValue,

    /// Run every day at HH:MM local time.
    #[arg(long)]
    pub at: Option<String>,

    /// Run at a fixed interval such as 30m or 6h.
    #[arg(long)]
    pub every: Option<String>,

    /// Run using a six-field expression: second minute hour day month weekday.
    #[arg(long)]
    pub cron: Option<String>,

    /// Select the whole workspace.
    #[arg(long)]
    pub all: bool,

    /// Select a repository by canonical name or workspace-relative directory.
    #[arg(long = "repo", value_name = "REPOSITORY")]
    pub repositories: Vec<String>,

    /// Behavior when the workspace is already locked.
    #[arg(long, value_enum, default_value = "skip")]
    pub overlap: ScheduleOverlapValue,

    /// Schedule timezone; currently only local is supported.
    #[arg(long, default_value = "local")]
    pub timezone: String,

    /// Add the declaration in a disabled state.
    #[arg(long)]
    pub disabled: bool,
}

#[derive(Debug, Args)]
#[command(
    group(clap::ArgGroup::new("trigger").args(["at", "every", "cron"])),
    group(clap::ArgGroup::new("scope").args(["all", "repositories"]))
)]
pub struct ScheduleUpdateArgs {
    pub name: String,

    /// Change the scheduled operation.
    #[arg(long, value_enum)]
    pub action: Option<ScheduleActionValue>,

    /// Replace the trigger with a daily HH:MM local time.
    #[arg(long)]
    pub at: Option<String>,

    /// Replace the trigger with a fixed interval such as 30m or 6h.
    #[arg(long)]
    pub every: Option<String>,

    /// Replace the trigger with a six-field cron expression.
    #[arg(long)]
    pub cron: Option<String>,

    /// Replace the scope with the whole workspace.
    #[arg(long)]
    pub all: bool,

    /// Replace the scope with these repositories.
    #[arg(long = "repo", value_name = "REPOSITORY")]
    pub repositories: Vec<String>,

    /// Change overlap behavior.
    #[arg(long, value_enum)]
    pub overlap: Option<ScheduleOverlapValue>,

    /// Change the timezone; currently only local is supported.
    #[arg(long)]
    pub timezone: Option<String>,

    /// Enable the schedule declaration.
    #[arg(long, conflicts_with = "disable")]
    pub enable: bool,

    /// Disable the schedule declaration.
    #[arg(long, conflicts_with = "enable")]
    pub disable: bool,
}

#[derive(Debug, Args)]
pub struct ScheduleRemoveArgs {
    pub name: String,

    /// Unregister the native task before removing the declaration.
    #[arg(long)]
    pub unregister: bool,

    /// Also remove local logs; requires --unregister.
    #[arg(long, requires = "unregister")]
    pub purge_history: bool,
}

#[derive(Debug, Args)]
pub struct ScheduleNameArgs {
    pub name: String,

    /// Emit structured JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ScheduleRunArgs {
    pub name: String,
}

#[derive(Debug, Args)]
pub struct ScheduleListArgs {
    /// Include local scheduler registration state.
    #[arg(long, visible_alias = "installed")]
    pub registered: bool,

    /// Emit structured JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum SchedulePlatform {
    Auto,
    Launchd,
    Systemd,
}

#[derive(Debug, Args)]
pub struct SchedulePlatformArgs {
    pub name: String,

    #[arg(long, value_enum, default_value = "auto")]
    pub platform: SchedulePlatform,
}

#[derive(Debug, Args)]
pub struct ScheduleDoctorArgs {
    pub name: Option<String>,

    #[arg(long, value_enum, default_value = "auto")]
    pub platform: SchedulePlatform,

    /// Emit structured JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ScheduleRegisterArgs {
    pub name: String,

    #[arg(long, value_enum, default_value = "auto")]
    pub platform: SchedulePlatform,

    /// Preview registration without changing the native scheduler.
    #[arg(long)]
    pub dry_run: bool,

    /// Move an existing registration to another scheduler platform.
    #[arg(long)]
    pub migrate: bool,

    /// Replace a colliding native task not recorded by batch-git.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct ScheduleUnregisterArgs {
    pub name: String,

    #[arg(long, value_enum, default_value = "auto")]
    pub platform: SchedulePlatform,

    /// Preview removal without changing the native scheduler.
    #[arg(long)]
    pub dry_run: bool,

    /// Also remove local schedule logs maintained by batch-git.
    #[arg(long)]
    pub purge_history: bool,
}

#[derive(Debug, Args)]
pub struct CheckoutArgs {
    /// Create a new branch instead of using the normal checkout resolution.
    #[arg(
        short = 'b',
        long = "create",
        conflicts_with_all = ["default", "feature"]
    )]
    pub create: bool,

    /// Branch to checkout or create.
    #[arg(
        required_unless_present_any = ["default", "feature"],
        conflicts_with_all = ["default", "feature"]
    )]
    pub branch: Option<String>,

    /// Checkout each repository's declared default branch.
    #[arg(
        short = 'd',
        long,
        conflicts_with_all = ["create", "from", "remote", "feature"]
    )]
    pub default: bool,

    /// Checkout the branch named by CURRENT_FEATURE_BRANCH.
    #[arg(short = 'f', long, visible_alias = "feat")]
    pub feature: bool,

    /// Start the new branch at this local branch, remote branch, tag, or commit.
    #[arg(long, requires = "create", conflicts_with = "default")]
    pub from: Option<String>,

    /// Select a remote when several contain the same branch.
    #[arg(long, conflicts_with = "default")]
    pub remote: Option<String>,
}

#[derive(Debug, Args)]
pub struct MergeArgs {
    /// Update the current branch from its upstream with fast-forward only before merging.
    #[arg(long, conflicts_with = "no_update_current")]
    pub update_current: bool,

    /// Do not update the current branch before merging.
    #[arg(long, conflicts_with = "update_current")]
    pub no_update_current: bool,

    /// Select a remote when several contain the same source branch.
    #[arg(long)]
    pub remote: Option<String>,

    /// Branch to merge into the current branch.
    pub branch: String,
}

#[derive(Debug, Args)]
pub struct ExecArgs {
    /// Repository name or workspace-relative directory.
    #[arg(required_unless_present = "matches")]
    pub selectors: Vec<String>,

    /// Select repository names using the same '*' wildcard as find.
    #[arg(long = "match", value_name = "PATTERN")]
    pub matches: Vec<String>,

    /// Arguments passed directly to Git after `--`.
    #[arg(last = true, required = true, allow_hyphen_values = true)]
    pub git_args: Vec<OsString>,
}

#[derive(Debug, Args)]
pub struct FindArgs {
    /// Branch pattern; '*' matches zero or more characters.
    pub pattern: String,

    /// Search local branches only.
    #[arg(long, conflicts_with = "remote")]
    pub local: bool,

    /// Search synchronized remote branches only.
    #[arg(long, conflicts_with = "local")]
    pub remote: bool,

    /// Limit repositories by name using the same '*' wildcard.
    #[arg(long, value_name = "PATTERN")]
    pub repo: Option<String>,

    /// Emit structured JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct InfoArgs {
    /// Repository name or workspace-relative directory.
    pub repository: Option<String>,

    /// Emit structured JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ForgetArgs {
    #[arg(required = true)]
    pub selectors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn help_commands_are_alphabetically_sorted() {
        let command = Cli::command();
        assert_sorted(command.get_subcommands().map(|command| command.get_name()));

        let schedule = command
            .get_subcommands()
            .find(|command| command.get_name() == "schedule")
            .expect("schedule subcommand");
        assert_sorted(schedule.get_subcommands().map(|command| command.get_name()));
    }

    fn assert_sorted<'a>(names: impl Iterator<Item = &'a str>) {
        let names: Vec<_> = names.collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }
}
