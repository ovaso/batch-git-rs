use clap::{Args, Subcommand};

/// Environment configuration inspection commands.
#[derive(Debug, Args)]
pub struct EnvArgs {
    #[command(subcommand)]
    pub command: EnvCommand,
}

#[derive(Debug, Subcommand)]
pub enum EnvCommand {
    /// List supported variables, defaults, and effective values.
    #[command(visible_alias = "ls")]
    List(EnvListArgs),
}

#[derive(Debug, Args)]
pub struct EnvListArgs {
    /// Include the description of each supported environment variable.
    #[arg(short = 'd', long)]
    pub description: bool,
}

/// Optional legacy `--json` switch for read-only commands that did not previously expose it.
#[derive(Debug, Args, Default)]
pub struct MachineReadableArgs {
    /// Emit the legacy structured JSON payload. Prefer global --output json for new integrations.
    #[arg(long)]
    pub json: bool,
}

/// A stable protocol document that can be discovered from the executable itself.
#[derive(Debug, Args)]
pub struct SchemaArgs {
    #[arg(value_enum)]
    pub document: SchemaDocument,
}

/// JSON Schema documents published by the executable.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum SchemaDocument {
    /// The v1 machine-output envelope used by --output json.
    OperationResult,
    /// The persisted workspace.toml data model expressed as JSON Schema.
    Workspace,
}
