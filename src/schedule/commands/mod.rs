//! Schedule command dispatch and shared execution policy.

mod declarations;
mod execution;
mod native;
mod query;
mod support;

use super::*;

#[cfg(test)]
pub(super) use execution::native_run_child_arguments;

/// Shared policy for one schedule command invocation.
///
/// Keeping the runtime and automation boundary in one value prevents each command family from
/// independently reconstructing receipt, revision, concurrency, and verbosity behavior.
#[derive(Clone, Copy)]
struct CommandContext<'a> {
    jobs: usize,
    verbose: bool,
    automation: &'a AutomationOptions,
}

impl CommandContext<'_> {
    /// Emit one schedule command result through the stable automation envelope.
    fn emit_data<T: Serialize>(
        &self,
        action: &str,
        root: &Path,
        exit_code: i32,
        data: &T,
    ) -> Result<()> {
        automation::emit_data(
            self.automation,
            &format!("schedule {action}"),
            Some(root),
            exit_code,
            data,
        )
    }

    /// Apply only while the workspace revision captured by the caller still matches.
    fn verify_apply_revision(&self, root: &Path) -> Result<()> {
        if let Some(expected) = &self.automation.expected_workspace_revision {
            workspace::verify_revision(root, expected)?;
        }
        Ok(())
    }
}

/// Route a schedule subcommand to its single-purpose command family.
pub(crate) fn dispatch(
    arguments: ScheduleArgs,
    jobs: usize,
    verbose: bool,
    automation: &AutomationOptions,
) -> Result<i32> {
    let context = CommandContext {
        jobs,
        verbose,
        automation,
    };
    match arguments.command {
        ScheduleCommand::Add(arguments) => declarations::add(arguments, &context),
        ScheduleCommand::Plan(arguments) => execution::plan(arguments, &context),
        ScheduleCommand::Run(arguments) => execution::run(arguments, &context),
        ScheduleCommand::List(arguments) => query::list(arguments, &context),
        ScheduleCommand::NativeRun(arguments) => execution::native_run(arguments, &context),
        ScheduleCommand::Status(arguments) => query::status(arguments, &context),
        ScheduleCommand::Doctor(arguments) => query::doctor(arguments, &context),
        ScheduleCommand::Generate(arguments) => native::generate(arguments, &context),
        ScheduleCommand::Register(arguments) => native::register(arguments, &context),
        ScheduleCommand::Remove(arguments) => declarations::remove(arguments, &context),
        ScheduleCommand::Unregister(arguments) => native::unregister(arguments, &context),
        ScheduleCommand::Update(arguments) => declarations::update(arguments, &context),
    }
}
