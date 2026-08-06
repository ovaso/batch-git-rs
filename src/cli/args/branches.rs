use clap::Args;

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
    #[arg(long, visible_alias = "uc", conflicts_with = "no_update_current")]
    pub update_current: bool,

    /// Do not update the current branch before merging; reject branches behind their upstream.
    #[arg(long, conflicts_with = "update_current")]
    pub no_update_current: bool,

    /// Refresh the source remote-tracking branch and merge it instead of a local source branch.
    #[arg(long, visible_alias = "rs", conflicts_with = "no_refresh_source")]
    pub refresh_source: bool,

    /// Do not refresh the source branch before merging.
    #[arg(long, conflicts_with = "refresh_source")]
    pub no_refresh_source: bool,

    /// Select a remote when several contain the same source branch.
    #[arg(long, conflicts_with = "default")]
    pub remote: Option<String>,

    /// Merge each repository's declared default branch into its current branch.
    #[arg(short = 'd', long)]
    pub default: bool,

    /// Merge the branch named by CURRENT_FEATURE_BRANCH.
    #[arg(long, conflicts_with_all = ["branch", "default"])]
    pub feature: bool,

    /// Source branch to merge into each repository's current branch.
    #[arg(
        required_unless_present_any = ["default", "feature"],
        conflicts_with_all = ["default", "feature"]
    )]
    pub branch: Option<String>,
}
