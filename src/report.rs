//! Stable, per-repository command results and rendering.

use std::io::{self, IsTerminal, Write};

use console::Term;
use unicode_width::UnicodeWidthStr;

use crate::git::GitOutput;
use crate::model::RepositoryRecord;
use crate::{color, table};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResultKind {
    Success,
    Skipped,
    Failed,
}

impl ResultKind {
    fn label(self) -> &'static str {
        match self {
            Self::Success => "ok",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    fn colored_label(self) -> String {
        match self {
            Self::Success => color::green(self.label()),
            Self::Skipped => self.label().to_owned(),
            Self::Failed => color::red(self.label()),
        }
    }

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

impl RepositoryResult {
    pub(crate) fn success(
        repository: &RepositoryRecord,
        detail: impl Into<String>,
        marks_synced: bool,
    ) -> Self {
        let mut result = Self::plain(repository, ResultKind::Success, detail);
        result.synced = marks_synced;
        result
    }

    pub(crate) fn skipped(repository: &RepositoryRecord, detail: impl Into<String>) -> Self {
        Self::plain(repository, ResultKind::Skipped, detail)
    }

    pub(crate) fn failed(repository: &RepositoryRecord, detail: impl Into<String>) -> Self {
        Self::plain(repository, ResultKind::Failed, detail)
    }

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

    pub(crate) fn was_synced(&self) -> bool {
        self.synced
    }

    pub(crate) fn is_failed(&self) -> bool {
        self.kind == ResultKind::Failed
    }

    pub(crate) fn progress_label(&self) -> String {
        match self.kind {
            ResultKind::Success => color::green("done"),
            ResultKind::Skipped => "verified".to_owned(),
            ResultKind::Failed => color::red("failed"),
        }
    }

    fn plain(repository: &RepositoryRecord, kind: ResultKind, detail: impl Into<String>) -> Self {
        Self {
            name: repository.name.clone(),
            directory: repository.directory.clone(),
            kind,
            detail: detail.into(),
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            synced: false,
        }
    }
}

pub(crate) fn print_operation_summary(results: &[RepositoryResult], verbose: bool) -> i32 {
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
    print_summary(results)
}

pub(crate) fn print_selected_results(results: &[RepositoryResult], verbose: bool) -> i32 {
    print_selected_results_with_comments(results, verbose, false)
}

pub(crate) fn print_push_summary(results: &[RepositoryResult], verbose: bool) -> i32 {
    print_selected_results_with_comments(results, verbose, true)
}

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

pub(crate) fn print_results(results: &[RepositoryResult], verbose: bool) -> i32 {
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

    print_summary(results)
}

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

pub(crate) fn print_checkout_summary(
    results: &[RepositoryResult],
    branches: &[String],
    default_branches: &[String],
    feature_branch: Option<&str>,
) -> i32 {
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
    print_summary(results)
}

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
struct OutputBlockStyle {
    unicode: bool,
    width: usize,
}

impl OutputBlockStyle {
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
