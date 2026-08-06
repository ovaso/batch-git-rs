# Command Organization Plan

[简体中文](zh-CN/COMMAND_ORGANIZATION_PLAN.md)

This plan covers future command-metadata and help-presentation work only. The current refactor does
not change CLI paths, arguments, exit codes, machine output, capability command strings, or schedule
behavior, and does not replace flat commands with nested forms such as `workspace scan` or
`repo pull`.

## Goals

- Centralize the facts that must be maintained when adding a command, reducing omissions across `Command`, mutability checks, dispatch, plan, and capabilities.
- Make top-level help easier to scan while preserving the flat command paths used by scripts, the automation contract, and the skill.
- Keep the safety distinctions between `fetch` / `sync` / `pull` and `list` / `status` / `info` explicit instead of obscuring side-effect boundaries by merging commands.

## Non-goals

- Do not rename, remove, or merge existing commands.
- Do not change how compatibility entry points such as `cd`, `cf`, and `cc` resolve.
- Do not change plan/apply, selectors, concurrency, exit codes, JSON/JSONL, reason codes, capabilities, or schemas.
- Do not introduce a second persistent state source for command classification; `workspace.toml` remains the only workspace source of truth.

## Implementation status and later phases

### 1. Freeze the public command surface

Use black-box tests to lock the top-level command paths and stable command strings in capabilities.
Every later implementation must first prove that existing invocations still parse and must run
`tests/automation_protocol.rs` plus the primary end-to-end workflows.

### 2. Single command metadata source (internal boundary implemented)

`cli::metadata` now centralizes, derives, or validates at compile time:

- canonical command strings and compatibility aliases;
- whether a command may have side effects and whether global `--plan` / `--apply` applies;
- top-level help categories;
- capabilities exposure;
- whether dispatch and plan have handling branches.

The model stores compile-time metadata only, never runtime state. clap argument types, dispatch, and
plan remain exhaustive matches so the compiler forces every new enum variant to be handled. Tests
verify that the clap top-level roster and metadata are identical. No dynamic registry was added.

### 3. Add a non-breaking help index

Add a top-level quick index without moving command paths:

- Workspace setup: `scan`, `clone`, `restore`, `forget`.
- Inspection: `list`, `status`, `info`, `branch`, `find`, `env`.
- Synchronization and remotes: `fetch`, `sync`, `pull`, `push`.
- Branches: `checkout`, `merge`.
- Staging and commits: `add`, `unstage`, `commit`.
- Automation: `capabilities`, `schema`, `schedule`.
- Escape hatches: `exec` and explicit `-- <git-args...>`.

The help index is navigation only; it does not change clap definitions, arguments, or ordering.
Implementation must update help snapshots, both language versions of the user guide, and both
changelogs without changing automation command strings.

### 4. Reduce shortcut noise

Evaluate hiding `cd` and `cf` from the default command list while continuing to accept direct
invocation. They remain equivalent to `checkout --default` and `checkout --feature`; they are not
removed or renamed, and preflight command normalization does not change. This phase requires
black-box coverage proving `cd` and `cf` remain callable and must evaluate shell-completion and error
message impact.

## Implementation gates

Every command-presentation change must check:

1. Top-level and subcommand `--help`.
2. `Command::is_mutating`, dispatch, plan, and capabilities coverage.
3. Whether both language versions of the user guide, automation contracts, changelog, and repository-local automation skill need updates.
4. CLI unit tests, `tests/automation_protocol.rs`, and `tests/mvp.rs`.
5. Linux, macOS, and Windows builds, including explicit scheduler differences.
