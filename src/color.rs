//! Terminal-aware semantic colors for human-readable output.

use std::io::{self, IsTerminal};

fn enabled() -> bool {
    io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").is_ok_and(|term| term != "dumb")
}

fn paint(value: impl std::fmt::Display, code: u8) -> String {
    if enabled() {
        // The invisible marker lets the table renderer distinguish trusted
        // styling from control characters originating in repository data.
        format!("\u{2063}\u{1b}[{code}m{value}\u{2063}\u{1b}[0m")
    } else {
        value.to_string()
    }
}

pub(crate) fn green(value: impl std::fmt::Display) -> String {
    paint(value, 32)
}

pub(crate) fn red(value: impl std::fmt::Display) -> String {
    paint(value, 31)
}

pub(crate) fn blue(value: impl std::fmt::Display) -> String {
    paint(value, 34)
}

pub(crate) fn magenta(value: impl std::fmt::Display) -> String {
    paint(value, 35)
}

pub(crate) fn yellow(value: impl std::fmt::Display) -> String {
    paint(value, 33)
}
