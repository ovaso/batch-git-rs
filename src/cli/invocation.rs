use crate::automation::{self, AutomationOptions, OutputFormat};
use anyhow::{Result, bail};
use std::ffi::OsString;

/// 在识别 Git 透传边界前允许出现的全局运行参数。
#[derive(Debug, Clone, Default)]
pub struct RuntimeOptions {
    /// 最大并发仓库数；未提供时稍后从环境变量或默认值解析。
    pub jobs: Option<usize>,
    /// 是否显示成功子进程的输出。
    pub verbose: bool,
    /// 新版机器输出协议。
    pub output: OutputFormat,
    /// 调用方用于关联日志和 receipt 的不透明标识。
    pub request_id: Option<String>,
    /// 禁止 Git 请求交互输入。
    pub non_interactive: bool,
    /// 子 Git 进程最长运行时间。
    pub timeout: Option<std::time::Duration>,
    /// 仅输出操作计划。
    pub plan: bool,
    /// 按计划前置条件执行操作。
    pub apply: bool,
    /// apply 必须匹配的 workspace.toml digest。
    pub expected_workspace_revision: Option<String>,
}

impl RuntimeOptions {
    /// 提取与输出和子进程执行有关的共享配置。
    pub fn automation(&self) -> AutomationOptions {
        AutomationOptions {
            output: self.output,
            request_id: self.request_id.clone(),
            non_interactive: self.non_interactive,
            timeout: self.timeout,
            plan: self.plan,
            apply: self.apply,
            expected_workspace_revision: self.expected_workspace_revision.clone(),
        }
    }
}

/// 一次调用要么进入内建命令解析器，要么原样透传给 Git。
pub enum Invocation {
    BuiltIn(Vec<OsString>),
    Passthrough {
        args: Vec<OsString>,
        options: RuntimeOptions,
    },
}

/// 在不损失 `OsString` 的前提下识别显式 `--` 命令边界。
pub fn parse_invocation(args: Vec<OsString>) -> Result<Invocation> {
    if args.is_empty() {
        bail!("missing argv[0]");
    }

    // 跳过 argv[0]，只手工消费透传模式也支持的少量全局选项。
    let mut index = 1;
    let mut options = RuntimeOptions::default();
    while index < args.len() {
        let value = args[index].to_string_lossy();
        if value == "--verbose" {
            options.verbose = true;
            index += 1;
        } else if value == "--jobs" {
            let Some(raw) = args.get(index + 1) else {
                bail!("--jobs requires a value");
            };
            options.jobs = Some(parse_jobs(raw)?);
            index += 2;
        } else if let Some(raw) = value.strip_prefix("--jobs=") {
            options.jobs = Some(parse_jobs(&OsString::from(raw))?);
            index += 1;
        } else if value == "--output" {
            let Some(raw) = args.get(index + 1) else {
                bail!("--output requires a value");
            };
            options.output = parse_output(&raw.to_string_lossy())?;
            index += 2;
        } else if let Some(raw) = value.strip_prefix("--output=") {
            options.output = parse_output(raw)?;
            index += 1;
        } else if value == "--request-id" {
            let Some(raw) = args.get(index + 1) else {
                bail!("--request-id requires a value");
            };
            options.request_id = Some(raw.to_string_lossy().into_owned());
            index += 2;
        } else if let Some(raw) = value.strip_prefix("--request-id=") {
            options.request_id = Some(raw.to_owned());
            index += 1;
        } else if value == "--timeout" {
            let Some(raw) = args.get(index + 1) else {
                bail!("--timeout requires a value");
            };
            options.timeout = Some(automation::parse_timeout(&raw.to_string_lossy())?);
            index += 2;
        } else if let Some(raw) = value.strip_prefix("--timeout=") {
            options.timeout = Some(automation::parse_timeout(raw)?);
            index += 1;
        } else if value == "--non-interactive" {
            options.non_interactive = true;
            index += 1;
        } else if value == "--plan" {
            options.plan = true;
            index += 1;
        } else if value == "--apply" {
            options.apply = true;
            index += 1;
        } else if value == "--expect-workspace-revision" {
            let Some(raw) = args.get(index + 1) else {
                bail!("--expect-workspace-revision requires a value");
            };
            options.expected_workspace_revision = Some(raw.to_string_lossy().into_owned());
            index += 2;
        } else if let Some(raw) = value.strip_prefix("--expect-workspace-revision=") {
            options.expected_workspace_revision = Some(raw.to_owned());
            index += 1;
        } else {
            break;
        }
    }

    // 只有显式分隔符才进入透传，未知内建命令不会被悄悄当作 Git 子命令。
    if args.get(index).is_some_and(|value| value == "--") {
        let passthrough = args[index + 1..].to_vec();
        if passthrough.is_empty() {
            bail!("Git passthrough requires arguments after --");
        }
        return Ok(Invocation::Passthrough {
            args: passthrough,
            options,
        });
    }

    Ok(Invocation::BuiltIn(args))
}

/// Best-effort global option scan used to render an early parse failure as JSON.
pub fn preflight_options(args: &[OsString]) -> AutomationOptions {
    let mut options = AutomationOptions::default();
    let mut index = 1;
    while index < args.len() {
        let value = args[index].to_string_lossy();
        if value == "--" {
            break;
        }
        if value == "--output" {
            if let Some(raw) = args.get(index + 1)
                && let Ok(output) = parse_output(&raw.to_string_lossy())
            {
                options.output = output;
            }
            index += 2;
            continue;
        }
        if let Some(raw) = value.strip_prefix("--output=") {
            if let Ok(output) = parse_output(raw) {
                options.output = output;
            }
        } else if value == "--request-id" {
            if let Some(raw) = args.get(index + 1) {
                options.request_id = Some(raw.to_string_lossy().into_owned());
            }
            index += 2;
            continue;
        } else if let Some(raw) = value.strip_prefix("--request-id=") {
            options.request_id = Some(raw.to_owned());
        }
        index += 1;
    }
    options
}

/// Guess the built-in command for a structured top-level error without touching passthrough args.
pub fn preflight_command(args: &[OsString]) -> Option<String> {
    let mut skip_next = false;
    let mut parent_command: Option<String> = None;
    for value in args.iter().skip(1) {
        let value = value.to_string_lossy();
        if value == "--" {
            return Some("passthrough".to_owned());
        }
        if skip_next {
            skip_next = false;
            continue;
        }
        if matches!(
            value.as_ref(),
            "--jobs" | "--output" | "--request-id" | "--timeout" | "--expect-workspace-revision"
        ) {
            skip_next = true;
            continue;
        }
        if value.starts_with("--") {
            continue;
        }
        if let Some(parent) = parent_command.as_deref() {
            let action = match parent {
                "schedule" => canonical_schedule_action(&value),
                "env" => canonical_env_action(&value),
                _ => unreachable!("only nested built-in commands are tracked"),
            };
            return Some(format!("{parent} {action}"));
        }
        let command = canonical_command(&value);
        if matches!(command, "schedule" | "env") {
            parent_command = Some(command.to_owned());
            continue;
        }
        return Some(command.to_owned());
    }
    parent_command
}

fn canonical_command(command: &str) -> &str {
    match command {
        "b" => "branch",
        "cc" | "cd" | "cf" => "checkout",
        "fd" | "f" => "find",
        "i" => "info",
        "ls" | "l" => "list",
        "m" => "merge",
        "s" => "status",
        other => other,
    }
}

fn canonical_schedule_action(action: &str) -> &str {
    match action {
        "create" => "add",
        "install" => "register",
        "delete" => "remove",
        "uninstall" => "unregister",
        "edit" => "update",
        other => other,
    }
}

fn canonical_env_action(action: &str) -> &str {
    match action {
        "ls" => "list",
        other => other,
    }
}

fn parse_output(raw: &str) -> Result<OutputFormat> {
    match raw {
        "text" => Ok(OutputFormat::Text),
        "json" => Ok(OutputFormat::Json),
        "jsonl" => Ok(OutputFormat::Jsonl),
        _ => bail!("invalid output format: {raw}; expected text, json, or jsonl"),
    }
}

/// 解析透传模式中的 `--jobs`，并提前拒绝零并发。
fn parse_jobs(value: &OsString) -> Result<usize> {
    let jobs = value
        .to_string_lossy()
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("invalid --jobs value: {}", value.to_string_lossy()))?;
    if jobs == 0 {
        bail!("--jobs must be at least 1");
    }
    Ok(jobs)
}
