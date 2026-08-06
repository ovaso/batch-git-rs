//! Compile-time metadata for the stable flat command surface.

/// One built-in command family, independent of its clap argument payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum CommandKind {
    Add,
    Branch,
    Capabilities,
    Cd,
    Cf,
    Checkout,
    Clone,
    Commit,
    Env,
    Exec,
    Fetch,
    Find,
    Forget,
    Info,
    List,
    Merge,
    Pull,
    Push,
    Restore,
    Scan,
    #[cfg(feature = "schedule")]
    Schedule,
    Schema,
    Status,
    Sync,
    Unstage,
    Count,
}

/// Facts shared by safety checks, automation discovery, and command-surface validation.
#[derive(Debug)]
pub(crate) struct CommandMetadata {
    pub(crate) name: &'static str,
    pub(crate) automation_name: &'static str,
    pub(crate) aliases: &'static [&'static str],
    pub(crate) mutating: bool,
    pub(crate) global_plan: bool,
    pub(crate) capability: bool,
}

const COMMANDS: [CommandMetadata; CommandKind::Count as usize] = [
    metadata("add", "add", &[], true, true, true),
    metadata("branch", "branch", &["b"], false, false, true),
    metadata("capabilities", "capabilities", &[], false, false, true),
    metadata("cd", "checkout", &[], true, true, false),
    metadata("cf", "checkout", &[], true, true, false),
    metadata("checkout", "checkout", &["cc"], true, true, true),
    metadata("clone", "clone", &[], true, true, true),
    metadata("commit", "commit", &[], true, true, true),
    metadata("env", "env", &[], false, false, true),
    metadata("exec", "exec", &[], true, true, true),
    metadata("fetch", "fetch", &[], true, true, true),
    metadata("find", "find", &["fd", "f"], false, false, true),
    metadata("forget", "forget", &[], true, true, true),
    metadata("info", "info", &["i"], false, false, true),
    metadata("list", "list", &["ls", "l"], false, false, true),
    metadata("merge", "merge", &["m"], true, true, true),
    metadata("pull", "pull", &[], true, true, true),
    metadata("push", "push", &[], true, true, true),
    metadata("restore", "restore", &[], true, true, true),
    metadata("scan", "scan", &[], true, true, true),
    #[cfg(feature = "schedule")]
    metadata("schedule", "schedule", &[], true, false, true),
    metadata("schema", "schema", &[], false, false, true),
    metadata("status", "status", &["s"], false, false, true),
    metadata("sync", "sync", &[], true, true, true),
    metadata("unstage", "unstage", &[], true, true, true),
];

const fn metadata(
    name: &'static str,
    automation_name: &'static str,
    aliases: &'static [&'static str],
    mutating: bool,
    global_plan: bool,
    capability: bool,
) -> CommandMetadata {
    CommandMetadata {
        name,
        automation_name,
        aliases,
        mutating,
        global_plan,
        capability,
    }
}

impl CommandKind {
    pub(crate) fn metadata(self) -> &'static CommandMetadata {
        &COMMANDS[self as usize]
    }
}

#[cfg(test)]
pub(crate) fn command_metadata() -> &'static [CommandMetadata] {
    &COMMANDS
}

pub(crate) fn command_capabilities() -> Vec<&'static str> {
    COMMANDS
        .iter()
        .filter(|command| command.capability)
        .map(|command| command.automation_name)
        .chain(std::iter::once("passthrough"))
        .collect()
}

pub(crate) fn canonical_command_name(command: &str) -> &str {
    COMMANDS
        .iter()
        .find(|metadata| metadata.name == command || metadata.aliases.contains(&command))
        .map_or(command, |metadata| metadata.automation_name)
}

#[cfg(test)]
mod tests {
    use super::COMMANDS;

    #[test]
    fn packaged_completions_cover_every_top_level_command() {
        let completions = [
            include_str!("../../completions/batch-git.bash"),
            include_str!("../../completions/_batch-git"),
            include_str!("../../completions/batch-git.fish"),
            include_str!("../../completions/batch-git.ps1"),
        ];
        for command in &COMMANDS {
            for completion in completions {
                assert!(
                    completion.contains(command.name),
                    "completion is missing top-level command {}",
                    command.name
                );
            }
        }
    }

    #[cfg(feature = "schedule")]
    #[test]
    fn packaged_completions_do_not_expose_internal_schedule_actions() {
        let completions = [
            include_str!("../../completions/batch-git.bash"),
            include_str!("../../completions/_batch-git"),
            include_str!("../../completions/batch-git.fish"),
            include_str!("../../completions/batch-git.ps1"),
        ];
        assert!(
            completions
                .iter()
                .all(|completion| !completion.contains("native-run"))
        );
    }
}
