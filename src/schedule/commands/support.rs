//! Pure schedule lookup, selection, and presentation helpers.

use super::*;

/// Find a schedule by its unique manifest name.
pub(super) fn find_schedule<'a>(manifest: &'a Workspace, name: &str) -> Result<&'a ScheduleRecord> {
    manifest
        .schedules
        .iter()
        .find(|schedule| schedule.name == name)
        .ok_or_else(|| anyhow::anyhow!("unknown schedule: {name}"))
}

/// Resolve a persisted schedule scope to command-layer repository records.
pub(super) fn selected_repositories(
    manifest: &Workspace,
    schedule: &ScheduleRecord,
) -> Result<Vec<crate::model::RepositoryRecord>> {
    crate::selector::select(
        manifest,
        &schedule.scope.repositories,
        &[],
        schedule.scope.all,
    )
}

/// Render the mutually exclusive trigger as stable compact text.
pub(super) fn trigger_label(schedule: &ScheduleRecord) -> String {
    schedule
        .at
        .as_ref()
        .map(|at| format!("daily {at}"))
        .or_else(|| {
            schedule
                .every
                .as_ref()
                .map(|every| format!("every {every}"))
        })
        .or_else(|| schedule.cron.as_ref().map(|cron| format!("cron {cron}")))
        .unwrap_or_else(|| "invalid".to_owned())
}

pub(super) fn overlap_label(overlap: ScheduleOverlap) -> &'static str {
    match overlap {
        ScheduleOverlap::Skip => "skip",
        ScheduleOverlap::Queue => "queue",
    }
}

pub(super) fn action_label(action: ScheduleAction) -> &'static str {
    match action {
        ScheduleAction::Sync => "sync",
        ScheduleAction::Pull => "pull",
    }
}
