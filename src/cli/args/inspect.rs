use clap::Args;

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
