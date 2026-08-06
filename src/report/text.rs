//! Human-readable result tables and summaries.

use std::path::Path;

use anyhow::Result;
use unicode_width::UnicodeWidthStr;

use crate::automation::AutomationOptions;
use crate::{color, table};

use super::child_output::{OutputBlockStyle, print_child_output, print_child_output_block};
use super::machine::print_machine_results;
use super::result::{RepositoryResult, ResultKind};

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
