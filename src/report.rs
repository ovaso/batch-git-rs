//! Stable, per-repository command results and rendering.

use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use console::Term;
use serde::Serialize;
use serde_json::json;
use unicode_width::UnicodeWidthStr;

use crate::automation::{AutomationOptions, OutputFormat};
use crate::git::GitOutput;
use crate::model::RepositoryRecord;
use crate::{color, table};

/// 单仓库操作的三态结果；跳过不等同于失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResultKind {
    Success,
    Skipped,
    Failed,
}

impl ResultKind {
    /// 返回稳定、无颜色的短标签。
    fn label(self) -> &'static str {
        match self {
            Self::Success => "ok",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    /// 为表格输出生成语义化颜色标签。
    fn colored_label(self) -> String {
        match self {
            Self::Success => color::green(self.label()),
            Self::Skipped => self.label().to_owned(),
            Self::Failed => color::red(self.label()),
        }
    }

    /// 为详细输出块选择 Unicode 或 ASCII 标记。
    fn block_label(self, unicode: bool) -> &'static str {
        match (self, unicode) {
            (Self::Success, true) => "✓ ok",
            (Self::Skipped, true) => "– skipped",
            (Self::Failed, true) => "✗ failed",
            (Self::Success, false) => "ok",
            (Self::Skipped, false) => "skipped",
            (Self::Failed, false) => "failed",
        }
    }

    fn colored_block_label(self, unicode: bool) -> String {
        match self {
            Self::Success => color::green(self.block_label(unicode)),
            Self::Skipped => self.block_label(unicode).to_owned(),
            Self::Failed => color::red(self.block_label(unicode)),
        }
    }
}

/// 一个仓库的业务结果、子进程输出和清单更新时间信息。
#[derive(Debug)]
pub(crate) struct RepositoryResult {
    name: String,
    directory: String,
    kind: ResultKind,
    detail: String,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    synced: bool,
}

#[derive(Serialize)]
struct MachineRepositoryResult<'a> {
    repository: &'a str,
    directory: &'a str,
    status: &'static str,
    detail: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    synchronized: bool,
}

#[derive(Serialize)]
struct MachineSummary {
    ok: usize,
    skipped: usize,
    failed: usize,
}

#[derive(Serialize)]
struct MachineBatchResult<'a> {
    summary: MachineSummary,
    results: Vec<MachineRepositoryResult<'a>>,
}

/// Serializes completion events from concurrent workers without delaying them until aggregation.
///
/// JSONL preserves the v1 manifest-order contract: out-of-order completions are buffered only
/// until their predecessors arrive, then the contiguous finished prefix is emitted immediately.
pub(crate) struct JsonlProgress<'a> {
    automation: &'a AutomationOptions,
    command: String,
    root: &'a Path,
    state: Mutex<JsonlProgressState>,
}

struct JsonlProgressState {
    next_index: usize,
    pending: BTreeMap<usize, serde_json::Value>,
}

impl<'a> JsonlProgress<'a> {
    /// Emit an immediate lifecycle start event when JSONL progress is requested.
    pub(crate) fn new(
        automation: &'a AutomationOptions,
        command: &str,
        root: &'a Path,
        repositories: usize,
    ) -> Result<Option<Self>> {
        if automation.output != OutputFormat::Jsonl {
            return Ok(None);
        }
        crate::automation::emit_event(
            automation,
            "started",
            command,
            Some(root),
            json!({"repositories": repositories}),
        )?;
        Ok(Some(Self {
            automation,
            command: command.to_owned(),
            root,
            state: Mutex::new(JsonlProgressState {
                next_index: 0,
                pending: BTreeMap::new(),
            }),
        }))
    }

    /// Emit a repository result at completion time. Serialization cannot fail for this fixed
    /// record shape; a poisoned stdout lock indicates the process is already unwinding.
    pub(crate) fn repository_finished(&self, index: usize, result: &RepositoryResult) {
        self.repository_finished_value(index, json!(result.machine()));
    }

    /// Emit a repository-shaped result supplied by a command that does not use `RepositoryResult`
    /// internally (for example, scan candidates before they exist in the manifest).
    pub(crate) fn repository_finished_value(&self, index: usize, result: serde_json::Value) {
        let data = json!({ "repository_index": index, "result": result });
        let mut state = self
            .state
            .lock()
            .expect("JSONL output lock must not be poisoned");
        state.pending.insert(index, data);
        while let Some(data) = {
            let next_index = state.next_index;
            state.pending.remove(&next_index)
        } {
            crate::automation::emit_event(
                self.automation,
                "repository_finished",
                &self.command,
                Some(self.root),
                data,
            )
            .expect("fixed JSONL repository event must serialize");
            state.next_index += 1;
        }
    }
}

impl RepositoryResult {
    /// 构造成功结果，并可标记该仓库已经完成远端同步。
    pub(crate) fn success(
        repository: &RepositoryRecord,
        detail: impl Into<String>,
        marks_synced: bool,
    ) -> Self {
        let mut result = Self::plain(repository, ResultKind::Success, detail);
        result.synced = marks_synced;
        result
    }

    /// 构造预期内跳过结果。
    pub(crate) fn skipped(repository: &RepositoryRecord, detail: impl Into<String>) -> Self {
        Self::plain(repository, ResultKind::Skipped, detail)
    }

    /// 构造没有子进程输出的失败结果。
    pub(crate) fn failed(repository: &RepositoryRecord, detail: impl Into<String>) -> Self {
        Self::plain(repository, ResultKind::Failed, detail)
    }

    /// 保留 Git 输出但把特定非零结果归类为跳过。
    pub(crate) fn skipped_from_git(
        repository: &RepositoryRecord,
        detail: impl Into<String>,
        output: GitOutput,
    ) -> Self {
        Self {
            name: repository.name.clone(),
            directory: repository.directory.clone(),
            kind: ResultKind::Skipped,
            detail: detail.into(),
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: None,
            synced: false,
        }
    }

    /// 按 Git 退出状态构造成功或失败结果。
    pub(crate) fn from_git(
        repository: &RepositoryRecord,
        output: GitOutput,
        success_detail: impl Into<String>,
        marks_synced: bool,
    ) -> Self {
        let kind = if output.success {
            ResultKind::Success
        } else {
            ResultKind::Failed
        };
        let detail = if output.success {
            success_detail.into()
        } else {
            format!("Git exited with status {}", output.code.unwrap_or(-1))
        };
        Self {
            name: repository.name.clone(),
            directory: repository.directory.clone(),
            kind,
            detail,
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: (!output.success).then_some(output.code.unwrap_or(-1)),
            synced: output.success && marks_synced,
        }
    }

    /// 判断调用方是否应更新清单中的 `synced_at`。
    pub(crate) fn was_synced(&self) -> bool {
        self.synced
    }

    /// 判断该结果是否应让聚合退出码变为 1。
    pub(crate) fn is_failed(&self) -> bool {
        self.kind == ResultKind::Failed
    }

    fn machine(&self) -> MachineRepositoryResult<'_> {
        MachineRepositoryResult {
            repository: &self.name,
            directory: &self.directory,
            status: self.kind.label(),
            detail: &self.detail,
            reason_code: self.reason_code(),
            exit_code: self.exit_code,
            synchronized: self.synced,
        }
    }

    fn reason_code(&self) -> Option<&'static str> {
        let detail = self.detail.to_ascii_lowercase();
        if detail.contains("working tree is not clean") {
            Some("dirty_worktree")
        } else if detail.contains("has no upstream") {
            Some("no_upstream")
        } else if detail.contains("branch is ambiguous") {
            Some("branch_ambiguous")
        } else if detail.contains("does not exist") {
            Some("branch_missing")
        } else if detail.contains("not materialized") || detail.contains("not a git repository") {
            Some("repository_unavailable")
        } else if detail.contains("nothing to push") {
            Some("nothing_to_push")
        } else if detail.contains("nothing to commit") {
            Some("nothing_to_commit")
        } else if detail.contains("timed out") {
            Some("timeout")
        } else if self.exit_code.is_some() {
            Some("git_exit")
        } else if self.kind == ResultKind::Failed {
            Some("operation_failed")
        } else {
            None
        }
    }

    /// 生成进度条结束时显示的短状态。
    pub(crate) fn progress_label(&self) -> String {
        match self.kind {
            ResultKind::Success => color::green("done"),
            ResultKind::Skipped => "verified".to_owned(),
            ResultKind::Failed => color::red("failed"),
        }
    }

    /// 构造不带 Git stdout/stderr 的基础结果。
    fn plain(repository: &RepositoryRecord, kind: ResultKind, detail: impl Into<String>) -> Self {
        let detail = crate::automation::sanitize_message(&detail.into());
        Self {
            name: repository.name.clone(),
            directory: repository.directory.clone(),
            kind,
            detail,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            synced: false,
        }
    }
}

/// 打印 clone/restore/fetch 风格摘要，并返回聚合退出码。
pub(crate) fn print_operation_summary(
    results: &[RepositoryResult],
    verbose: bool,
    automation: &AutomationOptions,
    command: &str,
    root: &Path,
) -> Result<i32> {
    if automation.is_machine() {
        return print_machine_results(results, automation, command, root);
    }
    let rows = results
        .iter()
        .map(|outcome| {
            let comment = if outcome.kind == ResultKind::Failed {
                outcome.detail.clone()
            } else {
                "-".to_owned()
            };
            vec![outcome.name.clone(), outcome.kind.colored_label(), comment]
        })
        .collect::<Vec<_>>();
    print!(
        "{}",
        table::render(&["REPOSITORY", "RESULT", "COMMENT"], &rows)
    );

    if verbose {
        for outcome in results
            .iter()
            .filter(|outcome| outcome.kind == ResultKind::Failed)
        {
            print_child_output(&outcome.stdout, &outcome.stderr, OutputBlockStyle::detect());
        }
    }
    Ok(print_summary(results))
}

/// 打印用户选中仓库的通用结果。
pub(crate) fn print_selected_results(
    results: &[RepositoryResult],
    verbose: bool,
    automation: &AutomationOptions,
    command: &str,
    root: &Path,
) -> Result<i32> {
    if automation.is_machine() {
        return print_machine_results(results, automation, command, root);
    }
    Ok(print_selected_results_with_comments(
        results, verbose, false,
    ))
}

/// 打印 push 结果，并显示“无 upstream”等跳过原因。
pub(crate) fn print_push_summary(
    results: &[RepositoryResult],
    verbose: bool,
    automation: &AutomationOptions,
    command: &str,
    root: &Path,
) -> Result<i32> {
    if automation.is_machine() {
        return print_machine_results(results, automation, command, root);
    }
    Ok(print_selected_results_with_comments(results, verbose, true))
}

/// 实现选中仓库结果表，并按策略附加详细输出块。
fn print_selected_results_with_comments(
    results: &[RepositoryResult],
    verbose: bool,
    show_skipped_detail: bool,
) -> i32 {
    let style = OutputBlockStyle::detect();
    let rows = results
        .iter()
        .map(|outcome| {
            let comment = if outcome.kind == ResultKind::Failed
                || (show_skipped_detail && outcome.kind == ResultKind::Skipped)
            {
                outcome.detail.clone()
            } else {
                "-".to_owned()
            };
            vec![outcome.name.clone(), outcome.kind.colored_label(), comment]
        })
        .collect::<Vec<_>>();
    print!(
        "{}",
        table::render(&["REPOSITORY", "RESULT", "COMMENT"], &rows)
    );

    for outcome in results
        .iter()
        .filter(|outcome| verbose || outcome.kind == ResultKind::Failed)
    {
        if outcome.stdout.is_empty() && outcome.stderr.is_empty() {
            continue;
        }
        println!();
        print_child_output_block(outcome, style, false, true);
    }
    print_summary(results)
}

/// 打印全工作区 Git 透传结果，成功输出默认可见。
pub(crate) fn print_results(
    results: &[RepositoryResult],
    verbose: bool,
    automation: &AutomationOptions,
    command: &str,
    root: &Path,
) -> Result<i32> {
    if automation.is_machine() {
        return print_machine_results(results, automation, command, root);
    }
    let style = OutputBlockStyle::detect();
    let visible_output = results
        .iter()
        .map(|outcome| {
            (verbose || outcome.kind == ResultKind::Failed)
                && (!outcome.stdout.is_empty() || !outcome.stderr.is_empty())
        })
        .collect::<Vec<_>>();
    let compact_width = results
        .iter()
        .zip(&visible_output)
        .filter(|(_, visible)| !**visible)
        .map(|(outcome, _)| UnicodeWidthStr::width(style.identity(outcome).as_str()))
        .max()
        .unwrap_or(0);
    let mut previous_was_block = false;

    for (index, (outcome, visible)) in results.iter().zip(visible_output).enumerate() {
        if visible {
            if index > 0 {
                println!();
            }
            print_child_output_block(outcome, style, true, true);
            previous_was_block = true;
        } else {
            if previous_was_block {
                println!();
            }
            println!("{}", style.compact_result(outcome, compact_width));
            previous_was_block = false;
        }
    }

    Ok(print_summary(results))
}

/// 输出成功/跳过/失败计数，并据此返回 0 或 1。
pub(crate) fn print_summary(results: &[RepositoryResult]) -> i32 {
    let mut succeeded = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for outcome in results {
        match outcome.kind {
            ResultKind::Success => succeeded += 1,
            ResultKind::Skipped => skipped += 1,
            ResultKind::Failed => failed += 1,
        }
    }

    println!();
    println!(
        "summary: {} ok, {skipped} skipped, {} failed",
        color::green(succeeded),
        color::red(failed)
    );
    i32::from(failed > 0)
}

/// 输出 checkout 专用摘要，把预期缺失分支的跳过计入成功退出。
pub(crate) fn print_checkout_summary(
    results: &[RepositoryResult],
    branches: &[String],
    default_branches: &[String],
    feature_branch: Option<&str>,
    automation: &AutomationOptions,
    command: &str,
    root: &Path,
) -> Result<i32> {
    if automation.is_machine() {
        return print_machine_results(results, automation, command, root);
    }
    debug_assert_eq!(results.len(), branches.len());
    debug_assert_eq!(results.len(), default_branches.len());
    let rows = results
        .iter()
        .zip(branches)
        .zip(default_branches)
        .map(|((outcome, branch), default_branch)| {
            vec![
                outcome.name.clone(),
                outcome.kind.colored_label(),
                color::branch(branch, default_branch, feature_branch),
                if outcome.kind == ResultKind::Failed {
                    outcome.detail.clone()
                } else {
                    "-".to_owned()
                },
            ]
        })
        .collect::<Vec<_>>();

    print!(
        "{}",
        table::render(
            &["REPOSITORY", "RESULT", "CURRENT BRANCH", "COMMENT"],
            &rows
        )
    );
    Ok(print_summary(results))
}

fn print_machine_results(
    results: &[RepositoryResult],
    automation: &AutomationOptions,
    command: &str,
    root: &Path,
) -> Result<i32> {
    let summary = machine_summary(results);
    let exit_code = i32::from(summary.failed > 0);
    if automation.output == OutputFormat::Jsonl {
        crate::automation::emit_finished(
            automation,
            command,
            Some(root),
            exit_code,
            serde_json::to_value(&summary)?,
        )?;
    } else {
        let data = MachineBatchResult {
            summary,
            results: results.iter().map(RepositoryResult::machine).collect(),
        };
        crate::automation::emit_data(automation, command, Some(root), exit_code, &data)?;
    }
    Ok(exit_code)
}

fn machine_summary(results: &[RepositoryResult]) -> MachineSummary {
    let mut summary = MachineSummary {
        ok: 0,
        skipped: 0,
        failed: 0,
    };
    for result in results {
        match result.kind {
            ResultKind::Success => summary.ok += 1,
            ResultKind::Skipped => summary.skipped += 1,
            ResultKind::Failed => summary.failed += 1,
        }
    }
    summary
}

/// 依次打印非空 stdout 和 stderr，并保留二者来源。
fn print_child_output(stdout: &str, stderr: &str, style: OutputBlockStyle) {
    if !stdout.is_empty() {
        print!("{stdout}");
        if !stdout.ends_with('\n') {
            println!();
        }
    }
    if !stderr.is_empty() {
        if !stdout.is_empty() {
            println!();
            println!("{}", style.stderr_separator());
        }
        let _ = io::stdout().flush();
        eprint!("{stderr}");
        if !stderr.ends_with('\n') {
            eprintln!();
        }
        let _ = io::stderr().flush();
    }
}

/// 为单仓库子进程输出添加可扫描的标题、边框和退出码说明。
fn print_child_output_block(
    outcome: &RepositoryResult,
    style: OutputBlockStyle,
    show_detail: bool,
    show_output: bool,
) {
    let has_captured_output = !outcome.stdout.is_empty() || !outcome.stderr.is_empty();
    let has_visible_output = show_output && has_captured_output;
    let note = output_block_note(outcome, show_detail, show_output, has_captured_output);
    println!("{}", style.header(outcome, note.as_deref()));
    if !has_visible_output {
        return;
    }
    print_child_output(&outcome.stdout, &outcome.stderr, style);
    println!("{}", style.footer());
}

/// 生成详细输出块右侧的状态说明。
fn output_block_note(
    outcome: &RepositoryResult,
    show_detail: bool,
    show_output: bool,
    has_captured_output: bool,
) -> Option<String> {
    if let Some(exit_code) = outcome.exit_code {
        return Some(format!("exit {exit_code}"));
    }
    if show_detail && outcome.kind != ResultKind::Success {
        return Some(outcome.detail.clone());
    }
    if !show_output && has_captured_output {
        return Some("output hidden".to_owned());
    }
    if !has_captured_output {
        return Some("no output".to_owned());
    }
    None
}

#[derive(Clone, Copy)]
/// 根据终端能力选择字符集和输出宽度。
struct OutputBlockStyle {
    unicode: bool,
    width: usize,
}

impl OutputBlockStyle {
    /// 检测终端是否支持 Unicode，并限制超宽输出。
    fn detect() -> Self {
        let unicode =
            io::stdout().is_terminal() && std::env::var("TERM").is_ok_and(|term| term != "dumb");
        let width = Term::stdout()
            .size_checked()
            .map(|(_, columns)| usize::from(columns))
            .unwrap_or(72)
            .clamp(48, 100);
        Self { unicode, width }
    }

    fn identity(self, outcome: &RepositoryResult) -> String {
        if outcome.name == outcome.directory {
            outcome.name.clone()
        } else if self.unicode {
            format!("{} · {}", outcome.name, outcome.directory)
        } else {
            format!("{} ({})", outcome.name, outcome.directory)
        }
    }

    fn compact_result(self, outcome: &RepositoryResult, identity_width: usize) -> String {
        let identity = self.identity(outcome);
        let padding = identity_width.saturating_sub(UnicodeWidthStr::width(identity.as_str()));
        let mut line = format!(
            "{identity}{}  {}",
            " ".repeat(padding),
            outcome.kind.colored_block_label(self.unicode)
        );
        let separator = if self.unicode { " · " } else { " - " };
        if let Some(exit_code) = outcome.exit_code {
            line.push_str(&format!("{separator}exit {exit_code}"));
        } else if outcome.kind != ResultKind::Success {
            line.push_str(&format!("{separator}{}", outcome.detail));
        }
        line
    }

    fn header(self, outcome: &RepositoryResult, note: Option<&str>) -> String {
        let identity = self.identity(outcome);
        let plain_status = outcome.kind.block_label(self.unicode);
        let colored_status = outcome.kind.colored_block_label(self.unicode);
        let note = note.unwrap_or_default();
        let (plain, rendered, fill) = if self.unicode {
            let suffix = if note.is_empty() {
                String::new()
            } else {
                format!(" · {note}")
            };
            (
                format!("╭─ {identity} · {plain_status}{suffix} "),
                format!("╭─ {identity} · {colored_status}{suffix} "),
                '─',
            )
        } else {
            let suffix = if note.is_empty() {
                String::new()
            } else {
                format!(" - {note}")
            };
            (
                format!("-- {identity} [{plain_status}]{suffix} "),
                format!("-- {identity} [{colored_status}]{suffix} "),
                '-',
            )
        };
        pad_rule(&plain, rendered, fill, self.width)
    }

    fn footer(self) -> String {
        if self.unicode {
            format!("╰{}", "─".repeat(self.width.saturating_sub(1)))
        } else {
            "-".repeat(self.width)
        }
    }

    fn stderr_separator(self) -> String {
        let (prefix, fill) = if self.unicode {
            ("┄ stderr ", '┄')
        } else {
            ("-- stderr ", '-')
        };
        pad_rule(prefix, prefix.to_owned(), fill, self.width)
    }
}

/// 在已着色标题后补齐横线，同时按无 ANSI 文本计算宽度。
fn pad_rule(plain: &str, rendered: String, fill: char, width: usize) -> String {
    let padding = width.saturating_sub(UnicodeWidthStr::width(plain));
    format!("{rendered}{}", fill.to_string().repeat(padding))
}

#[cfg(test)]
mod tests {
    use super::{OutputBlockStyle, RepositoryResult, output_block_note};
    use crate::git::GitOutput;
    use crate::model::RepositoryRecord;

    fn repository(name: &str, directory: &str) -> RepositoryRecord {
        RepositoryRecord {
            name: name.to_owned(),
            directory: directory.to_owned(),
            default_branch: "main".to_owned(),
            primary_remote: "origin".to_owned(),
            remotes: Vec::new(),
            created_at: "2026-07-29T00:00:00Z".to_owned(),
            synced_at: None,
        }
    }

    #[test]
    fn unicode_blocks_are_compact_and_avoid_duplicate_directories() {
        let style = OutputBlockStyle {
            unicode: true,
            width: 64,
        };
        let nested = RepositoryResult::success(
            &repository("service-api", "services/service-api"),
            "completed",
            false,
        );
        assert!(
            style
                .header(&nested, None)
                .starts_with("╭─ service-api · services/service-api · ✓ ok ")
        );

        let flat = RepositoryResult::success(
            &repository("service-api", "service-api"),
            "completed",
            false,
        );
        let header = style.header(&flat, None);
        assert!(header.starts_with("╭─ service-api · ✓ ok "));
        assert!(!header.contains("service-api · service-api"));
        assert_eq!(style.footer(), format!("╰{}", "─".repeat(63)));
    }

    #[test]
    fn ascii_blocks_show_git_exit_codes() {
        let repository = repository("service-api", "services/service-api");
        let outcome = RepositoryResult::from_git(
            &repository,
            GitOutput {
                success: false,
                code: Some(128),
                stdout: String::new(),
                stderr: "fatal\n".to_owned(),
            },
            "completed",
            false,
        );
        let note = output_block_note(&outcome, true, true, true);
        let header = OutputBlockStyle {
            unicode: false,
            width: 72,
        }
        .header(&outcome, note.as_deref());
        assert!(header.starts_with("-- service-api (services/service-api) [failed] - exit 128 "));
        assert_eq!(
            OutputBlockStyle {
                unicode: false,
                width: 72,
            }
            .compact_result(&outcome, "service-api (services/service-api)".len()),
            "service-api (services/service-api)  failed - exit 128"
        );
    }
}
