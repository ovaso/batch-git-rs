//! Command-line parsing and the explicit `--` passthrough boundary.

mod args;
mod command;
mod invocation;
mod metadata;

#[allow(unused_imports)]
// The facade intentionally keeps every command argument type at `crate::cli::*`.
pub use args::*;
pub use command::{Cli, Command};
pub use invocation::{
    Invocation, RuntimeOptions, parse_invocation, preflight_command, preflight_options,
};
#[cfg(test)]
pub(crate) use metadata::command_metadata;
pub(crate) use metadata::{canonical_command_name, command_capabilities};
