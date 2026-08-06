//! Git repository inspection through `git2` and compatibility-sensitive operations through Git.

mod checkout;
mod clone;
mod discovery;
mod execution;
mod inspect;
mod remotes;
mod types;

pub use checkout::{
    checkout_local, checkout_remote, checkout_target, create_and_checkout_branch,
    remote_tracking_target,
};
#[allow(unused_imports)] // Compatibility wrapper retained for crate-internal callers.
pub use clone::clone_repository;
pub use clone::clone_repository_with_options;
pub use discovery::discover;
#[allow(unused_imports)] // Compatibility wrappers retained for crate-internal callers.
pub use execution::{run, run_os, run_os_with_options, run_with_options};
pub use inspect::{
    branches, current_branch_summary, head_is_unborn, inspect, is_repository,
    operation_in_progress, repository_runtime_info, staging_state, status_summary,
};
#[allow(unused_imports)] // Compatibility wrapper retained for crate-internal callers.
pub use remotes::fetch_all;
pub use remotes::{
    configure_declared_remotes, display_remote_url, fetch_all_with_options, upstream_push_target,
    verify_declared_remotes,
};
#[allow(unused_imports)]
// Stable facade also exposes types not used by the binary crate itself.
pub use types::{
    BranchKind, BranchSummary, ChangeCounts, CheckoutTarget, CloneOptions, CloneProgress,
    GitExecutionOptions, GitOutput, RepositoryInfo, RepositoryRuntimeInfo, RepositoryRuntimeState,
    RepositoryStatusSummary, StagingState, UpstreamSummary,
};
