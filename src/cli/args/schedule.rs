use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct ScheduleArgs {
    #[command(subcommand)]
    pub command: ScheduleCommand,
}

#[derive(Debug, Subcommand)]
pub enum ScheduleCommand {
    /// Add a schedule declaration to batchspace.toml.
    #[command(visible_alias = "create")]
    Add(ScheduleAddArgs),
    /// Validate a schedule and the selected platform.
    Doctor(ScheduleDoctorArgs),
    /// Generate a native scheduler definition without registering it.
    Generate(SchedulePlatformArgs),
    /// List declared schedules.
    List(ScheduleListArgs),
    #[command(hide = true)]
    NativeRun(ScheduleNativeRunArgs),
    /// Preview the repositories and action selected by a schedule.
    Plan(ScheduleNameArgs),
    /// Create or update a native scheduler task.
    #[command(visible_alias = "install")]
    Register(ScheduleRegisterArgs),
    /// Remove a schedule declaration from batchspace.toml.
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

impl ScheduleCommand {
    pub(in crate::cli) fn is_mutating(&self) -> bool {
        !matches!(
            self,
            Self::Doctor(_) | Self::Generate(_) | Self::List(_) | Self::Plan(_) | Self::Status(_)
        )
    }
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

    /// Run every day at HH:MM in BATCH_GIT_TZ or the system timezone.
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

    /// Replace the daily trigger time.
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
pub struct ScheduleNativeRunArgs {
    pub name: String,

    #[arg(long)]
    pub log: bool,

    #[arg(long)]
    pub timezone: Option<String>,
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
    Windows,
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
