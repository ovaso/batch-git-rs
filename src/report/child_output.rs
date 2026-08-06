use std::io::{self, IsTerminal, Write};

use console::Term;
use unicode_width::UnicodeWidthStr;

use super::result::{RepositoryResult, ResultKind};

/// 依次打印非空 stdout 和 stderr，并保留二者来源。
pub(super) fn print_child_output(stdout: &str, stderr: &str, style: OutputBlockStyle) {
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
pub(super) fn print_child_output_block(
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
pub(super) struct OutputBlockStyle {
    unicode: bool,
    width: usize,
}

impl OutputBlockStyle {
    /// 检测终端是否支持 Unicode，并限制超宽输出。
    pub(super) fn detect() -> Self {
        let unicode =
            io::stdout().is_terminal() && std::env::var("TERM").is_ok_and(|term| term != "dumb");
        let width = Term::stdout()
            .size_checked()
            .map(|(_, columns)| usize::from(columns))
            .unwrap_or(72)
            .clamp(48, 100);
        Self { unicode, width }
    }

    pub(super) fn identity(self, outcome: &RepositoryResult) -> String {
        if outcome.name == outcome.directory {
            outcome.name.clone()
        } else if self.unicode {
            format!("{} · {}", outcome.name, outcome.directory)
        } else {
            format!("{} ({})", outcome.name, outcome.directory)
        }
    }

    pub(super) fn compact_result(
        self,
        outcome: &RepositoryResult,
        identity_width: usize,
    ) -> String {
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
