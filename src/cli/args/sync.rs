use clap::Args;

/// 多个命令共享的仓库选择参数。
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

/// 安全批量 push 的参数。
#[derive(Debug, Args)]
pub struct PushArgs {
    #[command(flatten)]
    pub selection: SyncArgs,

    /// Create the remote branch and configure upstream when it is missing.
    #[arg(short = 'u', long)]
    pub set_upstream: bool,

    /// Remote used with --set-upstream; defaults to the repository's primary remote.
    #[arg(long, requires = "set_upstream")]
    pub remote: Option<String>,

    /// Preview the push without updating the remote.
    #[arg(long)]
    pub dry_run: bool,
}

/// Commit the index of each selected repository with one shared message.
#[derive(Debug, Args)]
pub struct CommitArgs {
    #[command(flatten)]
    pub selection: SyncArgs,

    /// Commit message used in every repository that has staged changes.
    #[arg(short = 'm', long)]
    pub message: String,
}
