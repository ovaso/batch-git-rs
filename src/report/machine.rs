use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::automation::{AutomationOptions, OutputFormat};

use super::result::{RepositoryResult, ResultKind};

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

pub(super) fn repository_value(result: &RepositoryResult) -> serde_json::Value {
    serde_json::json!(machine_result(result))
}

fn machine_result(result: &RepositoryResult) -> MachineRepositoryResult<'_> {
    MachineRepositoryResult {
        repository: &result.name,
        directory: &result.directory,
        status: result.kind.label(),
        detail: &result.detail,
        reason_code: result.reason_code(),
        exit_code: result.exit_code,
        synchronized: result.synced,
    }
}

pub(super) fn print_machine_results(
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
            results: results.iter().map(machine_result).collect(),
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
