//! Core library for the `batch-git` executable.
#![forbid(unsafe_code)]

mod automation;
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

/// 执行一次完整的命令行调用，并把顶层错误统一转换为退出码 `2`。
pub fn run(args: Vec<OsString>) -> i32 {
    let fallback = cli::preflight_options(&args);
    let command = cli::preflight_command(&args);
    // 业务层使用 `Result` 保留错误上下文，这里是唯一的顶层错误展示边界。
    match run_inner(args, &fallback) {
        Ok(code) => code,
        Err(error) => {
            if fallback.is_machine() {
                if let Err(render_error) =
                    automation::emit_error(&fallback, command.as_deref(), &error)
                {
                    eprintln!("error: {render_error:#}");
                }
                return 2;
            }
            // `{error:#}` 会展开 anyhow 上下文链，展示具体失败阶段。
            eprintln!("{}: {error:#}", color::red("error"));
            2
        }
    }
}

fn run_inner(args: Vec<OsString>, fallback: &automation::AutomationOptions) -> anyhow::Result<i32> {
    // 先识别 `batch-git -- <git args>`，再让 clap 解析内建命令，避免歧义。
    match cli::parse_invocation(args)? {
        cli::Invocation::Passthrough { args, options } => commands::passthrough(args, options),
        cli::Invocation::BuiltIn(args) => match cli::Cli::try_parse_from(args) {
            Ok(cli) => commands::dispatch(cli),
            Err(error) => {
                // help/version 使用 stdout 并成功退出；参数错误使用 stderr 和退出码 2。
                let code = if error.use_stderr() { 2 } else { 0 };
                if fallback.is_machine() && error.use_stderr() {
                    return Err(anyhow::anyhow!("invalid arguments: {error}"));
                }
                // clap 已经生成完整诊断文本，因此不再用 anyhow 重复包装。
                let _ = error.print();
                Ok(code)
            }
        },
    }
}
