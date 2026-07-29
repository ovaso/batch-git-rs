//! Core library for the `batch-git` executable.
#![forbid(unsafe_code)]

mod cli;
mod color;
mod commands;
mod cron;
mod git;
mod model;
mod parallel;
mod report;
mod schedule;
mod selector;
mod settings;
mod table;
mod workspace;

use std::ffi::OsString;

use clap::Parser;

pub fn run(args: Vec<OsString>) -> i32 {
    match run_inner(args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{}: {error:#}", color::red("error"));
            2
        }
    }
}

fn run_inner(args: Vec<OsString>) -> anyhow::Result<i32> {
    match cli::parse_invocation(args)? {
        cli::Invocation::Passthrough { args, options } => commands::passthrough(args, options),
        cli::Invocation::BuiltIn(args) => match cli::Cli::try_parse_from(args) {
            Ok(cli) => commands::dispatch(cli),
            Err(error) => {
                let code = if error.use_stderr() { 2 } else { 0 };
                let _ = error.print();
                Ok(code)
            }
        },
    }
}
