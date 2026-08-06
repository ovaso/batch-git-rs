//! Command-line parsing and the explicit `--` passthrough boundary.

mod args;
mod command;
mod invocation;

#[allow(unused_imports)]
// The facade intentionally keeps every command argument type at `crate::cli::*`.
pub use args::*;
pub use command::{Cli, Command};
pub use invocation::{
    Invocation, RuntimeOptions, parse_invocation, preflight_command, preflight_options,
};
