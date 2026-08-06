//! Small Unicode-aware renderer for human-readable command output.

use std::io::{self, IsTerminal};

use console::Term;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// 相邻列之间固定保留两个空格。
const COLUMN_GAP: usize = 2;

/// 把表头和数据行渲染成 Unicode 宽度正确的左对齐表格。
pub(crate) fn render(headers: &[&str], rows: &[Vec<String>]) -> String {
    render_with_width(headers, rows, terminal_width())
}

fn render_with_width(
    headers: &[&str],
    rows: &[Vec<String>],
    maximum_width: Option<usize>,
) -> String {
    let headers = headers
        .iter()
        .map(|header| sanitize_cell(header))
        .collect::<Vec<_>>();
    let rows = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| sanitize_cell(cell))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    // 宽度使用终端可见字符计算，不能直接使用 UTF-8 字节数。
    let mut widths = headers
        .iter()
        .map(|header| visible_width(header))
        .collect::<Vec<_>>();

    for row in &rows {
        debug_assert_eq!(row.len(), headers.len());
        for (index, value) in row.iter().enumerate() {
            widths[index] = widths[index].max(visible_width(value));
        }
    }
    if let Some(maximum_width) = maximum_width {
        widths = fit_widths(&widths, &headers, maximum_width);
    }

    let mut output = String::new();
    push_wrapped_row(&mut output, &headers, &widths);
    let separator: Vec<String> = widths.iter().map(|width| "-".repeat(*width)).collect();
    push_wrapped_row(&mut output, &separator, &widths);
    for row in &rows {
        push_wrapped_row(&mut output, row, &widths);
    }
    output
}

fn terminal_width() -> Option<usize> {
    if !io::stdout().is_terminal() {
        return None;
    }
    Term::stdout()
        .size_checked()
        .map(|(_, columns)| usize::from(columns))
        .filter(|columns| *columns > 0)
}

/// 在终端宽度内为各列分配空间；优先容纳表头，再公平扩展到内容自然宽度。
fn fit_widths(natural: &[usize], headers: &[String], maximum_width: usize) -> Vec<usize> {
    if natural.is_empty() {
        return Vec::new();
    }
    let gap_width = COLUMN_GAP.saturating_mul(natural.len().saturating_sub(1));
    if natural.iter().sum::<usize>() + gap_width <= maximum_width {
        return natural.to_vec();
    }

    let available = maximum_width.saturating_sub(gap_width).max(natural.len());
    let header_widths = headers
        .iter()
        .zip(natural)
        .map(|(header, natural)| visible_width(header).max(1).min(*natural))
        .collect::<Vec<_>>();
    let mut widths = vec![1; natural.len()];
    let mut remaining = available.saturating_sub(widths.len());
    grow_widths(&mut widths, &header_widths, &mut remaining);
    grow_widths(&mut widths, natural, &mut remaining);
    widths
}

fn grow_widths(widths: &mut [usize], targets: &[usize], remaining: &mut usize) {
    while *remaining > 0 {
        let mut grew = false;
        for (width, target) in widths.iter_mut().zip(targets) {
            if *remaining == 0 {
                break;
            }
            if *width < *target {
                *width += 1;
                *remaining -= 1;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
}

/// 转义不可信控制字符，同时保留本程序自己生成的颜色序列。
fn sanitize_cell(value: &str) -> String {
    let mut characters = value.chars().peekable();
    let mut output = String::new();
    while let Some(character) = characters.next() {
        if character == '\u{2063}' && characters.peek() == Some(&'\u{1b}') {
            let mut sequence = String::from("\u{1b}[");
            characters.next();
            if characters.next() != Some('[') {
                output.push_str("\\u{2063}\\u{1b}");
                continue;
            }
            while let Some(&next) = characters.peek() {
                if next.is_ascii_digit() || next == ';' {
                    sequence.push(next);
                    characters.next();
                } else {
                    break;
                }
            }
            if characters.peek() == Some(&'m') {
                sequence.push('m');
                characters.next();
                output.push_str(&sequence);
                continue;
            }
            output.push_str("\\u{2063}\\u{1b}[");
            output.push_str(&sequence[2..]);
            continue;
        }
        match character {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                output.push_str(&format!("\\u{{{:x}}}", character as u32));
            }
            character => output.push(character),
        }
    }
    output
}

/// 去除 ANSI 序列后计算字符串在终端中占用的列数。
fn visible_width(value: &str) -> usize {
    let mut plain = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\u{1b}' && characters.peek() == Some(&'[') {
            characters.next();
            for next in characters.by_ref() {
                if next == 'm' {
                    break;
                }
            }
        } else {
            plain.push(character);
        }
    }
    UnicodeWidthStr::width(plain.as_str())
}

/// 按列宽对单元格软换行；续行中的空单元格仍占位，使内容从原列起点继续。
fn push_wrapped_row(output: &mut String, values: &[String], widths: &[usize]) {
    let cells = values
        .iter()
        .zip(widths)
        .map(|(value, width)| wrap_cell(value, *width))
        .collect::<Vec<_>>();
    let height = cells.iter().map(Vec::len).max().unwrap_or(1);
    for line in 0..height {
        let last_visible = cells
            .iter()
            .rposition(|cell| cell.get(line).is_some_and(|value| !value.is_empty()))
            .unwrap_or(0);
        for (index, (cell, width)) in cells.iter().zip(widths).enumerate().take(last_visible + 1) {
            let value = cell.get(line).map(String::as_str).unwrap_or("");
            output.push_str(value);
            if index < last_visible {
                let padding = width.saturating_sub(visible_width(value)) + COLUMN_GAP;
                output.push_str(&" ".repeat(padding));
            }
        }
        output.push('\n');
    }
}

/// 按终端可见宽度拆分单元格，并在换行处闭合、恢复本程序生成的 ANSI 样式。
fn wrap_cell(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = vec![String::new()];
    let mut line_width = 0;
    let mut active_style = String::new();
    let mut characters = value.chars().peekable();

    while let Some(character) = characters.next() {
        if character == '\u{1b}' && characters.peek() == Some(&'[') {
            let mut sequence = String::from("\u{1b}[");
            characters.next();
            for next in characters.by_ref() {
                sequence.push(next);
                if next == 'm' {
                    break;
                }
            }
            lines
                .last_mut()
                .expect("one line exists")
                .push_str(&sequence);
            if matches!(sequence.as_str(), "\u{1b}[0m" | "\u{1b}[m") {
                active_style.clear();
            } else if sequence.ends_with('m') {
                active_style.push_str(&sequence);
            }
            continue;
        }

        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if character_width > 0 && line_width > 0 && line_width + character_width > width {
            if !active_style.is_empty() {
                lines
                    .last_mut()
                    .expect("one line exists")
                    .push_str("\u{1b}[0m");
            }
            lines.push(active_style.clone());
            line_width = 0;
        }
        lines.last_mut().expect("one line exists").push(character);
        line_width += character_width;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::{render, render_with_width, visible_width};

    #[test]
    fn aligns_ascii_and_wide_characters() {
        let table = render(
            &["NAME", "BRANCH"],
            &[
                vec!["short".to_owned(), "master".to_owned()],
                vec!["long-project".to_owned(), "中文分支".to_owned()],
            ],
        );
        assert_eq!(
            table,
            "NAME          BRANCH\n------------  --------\nshort         master\nlong-project  中文分支\n"
        );
    }

    #[test]
    fn escapes_control_characters_without_breaking_alignment() {
        let table = render(
            &["NAME", "BRANCH"],
            &[
                vec!["api\tworker".to_owned(), "功能/登录".to_owned()],
                vec!["web\nui".to_owned(), "\u{1b}[31mmain".to_owned()],
            ],
        );
        assert_eq!(
            table,
            "NAME         BRANCH\n-----------  --------------\napi\\tworker  功能/登录\nweb\\nui      \\u{1b}[31mmain\n"
        );
    }

    #[test]
    fn ansi_colors_do_not_affect_alignment() {
        let table = render(
            &["NAME", "RESULT"],
            &[
                vec![
                    "api".to_owned(),
                    "\u{2063}\u{1b}[32mok\u{2063}\u{1b}[0m".to_owned(),
                ],
                vec![
                    "worker".to_owned(),
                    "\u{2063}\u{1b}[31mfailed\u{2063}\u{1b}[0m".to_owned(),
                ],
            ],
        );
        assert_eq!(
            table,
            "NAME    RESULT\n------  ------\napi     \u{1b}[32mok\u{1b}[0m\nworker  \u{1b}[31mfailed\u{1b}[0m\n"
        );
    }

    #[test]
    fn wraps_long_cells_at_their_column_start() {
        let table = render_with_width(
            &["NAME", "VALUE"],
            &[vec![
                "service".to_owned(),
                "abcdefghijklmnopqrstuvwxyz".to_owned(),
            ]],
            Some(18),
        );
        assert_eq!(
            table,
            "NAME     VALUE\n-------  ---------\nservice  abcdefghi\n         jklmnopqr\n         stuvwxyz\n"
        );
        assert!(table.lines().all(|line| visible_width(line) <= 18));
    }

    #[test]
    fn preserves_ansi_colors_across_wrapped_lines() {
        let table = render_with_width(
            &["NAME", "CURRENT"],
            &[vec![
                "service".to_owned(),
                "\u{2063}\u{1b}[32mabcdefghijklmnop\u{2063}\u{1b}[0m".to_owned(),
            ]],
            Some(18),
        );
        assert!(table.contains("service  \u{1b}[32mabcdefghi\u{1b}[0m\n"));
        assert!(table.contains("         \u{1b}[32mjklmnop\u{1b}[0m\n"));
        assert!(table.lines().all(|line| visible_width(line) <= 18));
    }
}
