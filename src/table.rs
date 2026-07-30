//! Small Unicode-aware renderer for human-readable command output.

use unicode_width::UnicodeWidthStr;

/// 相邻列之间固定保留两个空格。
const COLUMN_GAP: usize = 2;

/// 把表头和数据行渲染成 Unicode 宽度正确的左对齐表格。
pub(crate) fn render(headers: &[&str], rows: &[Vec<String>]) -> String {
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

    let mut output = String::new();
    push_row(&mut output, headers.iter().map(String::as_str), &widths);
    let separator: Vec<String> = widths.iter().map(|width| "-".repeat(*width)).collect();
    push_row(&mut output, separator.iter().map(String::as_str), &widths);
    for row in &rows {
        push_row(&mut output, row.iter().map(String::as_str), &widths);
    }
    output
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

/// 按预先计算的宽度写入一行，并在列间补齐空格。
fn push_row<'a, I>(output: &mut String, values: I, widths: &[usize])
where
    I: IntoIterator<Item = &'a str>,
{
    let values: Vec<&str> = values.into_iter().collect();
    for (index, value) in values.iter().enumerate() {
        output.push_str(value);
        if index + 1 < values.len() {
            let padding = widths[index] - visible_width(value) + COLUMN_GAP;
            output.push_str(&" ".repeat(padding));
        }
    }
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use super::render;

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
}
