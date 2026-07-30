//! Terminal-aware semantic colors for human-readable output.

use std::io::{self, IsTerminal};

/// 判断 stdout 是否适合写入 ANSI 颜色序列。
fn enabled() -> bool {
    // 尊重 NO_COLOR 约定，并在管道输出或弱终端中保持纯文本。
    io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").is_ok_and(|term| term != "dumb")
}

/// 用 ANSI 色号渲染内容，禁用颜色时返回原始可见文本。
fn paint(value: impl std::fmt::Display, code: u8) -> String {
    if enabled() {
        // The invisible marker lets the table renderer distinguish trusted
        // styling from control characters originating in repository data.
        format!("\u{2063}\u{1b}[{code}m{value}\u{2063}\u{1b}[0m")
    } else {
        value.to_string()
    }
}

/// 渲染成功状态。
pub(crate) fn green(value: impl std::fmt::Display) -> String {
    paint(value, 32)
}

/// 渲染失败状态。
pub(crate) fn red(value: impl std::fmt::Display) -> String {
    paint(value, 31)
}

/// 渲染主要信息。
pub(crate) fn blue(value: impl std::fmt::Display) -> String {
    paint(value, 34)
}

/// 渲染特性分支等强调信息。
pub(crate) fn magenta(value: impl std::fmt::Display) -> String {
    paint(value, 35)
}

/// 渲染警告或跳过状态。
pub(crate) fn yellow(value: impl std::fmt::Display) -> String {
    paint(value, 33)
}

/// 根据默认分支、当前特性分支或普通分支的角色选择颜色。
pub(crate) fn branch(value: &str, default_branch: &str, feature_branch: Option<&str>) -> String {
    if Some(value) == feature_branch {
        magenta(value)
    } else if value == default_branch {
        blue(value)
    } else {
        value.to_owned()
    }
}
