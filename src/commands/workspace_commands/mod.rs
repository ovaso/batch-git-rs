//! Workspace discovery, materialization, and manifest membership command facade.

mod clone;
mod membership;
mod restore;
mod scan;
mod support;

pub(super) use clone::{clone_repository, default_clone_directory};
pub(super) use membership::forget;
pub(super) use restore::{restore, restore_one};
pub(super) use scan::scan;
pub(super) use support::relative_string;
