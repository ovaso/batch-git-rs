use std::ffi::OsString;

use clap::Args;

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
