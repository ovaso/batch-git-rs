use std::path::PathBuf;

use clap::Args;

/// 受控 clone 的参数集合。
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

/// 扫描已有目录的参数。
#[derive(Debug, Args)]
pub struct ScanArgs {
    /// Maximum directory depth to scan; defaults to BATCH_GIT_SCAN_DEPTH or 1.
    #[arg(short = 'd', long)]
    pub depth: Option<usize>,
}

#[derive(Debug, Args)]
pub struct ForgetArgs {
    #[arg(required = true)]
    pub selectors: Vec<String>,
}
