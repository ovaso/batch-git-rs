# Architecture

[简体中文](zh-CN/ARCHITECTURE.md)

## Goals and boundaries

`batch-git` coordinates multiple independent Git working trees; it does not create a monorepo.
`batchspace.toml` stores portable restoration declarations. Current branches, HEAD, working-tree
changes, and remote references are always read live from local Git repositories.

```text
CLI / env ──> cli + settings + automation ──> commands / schedule
                                  │
                     workspace ──┼── model (batchspace.toml validation)
                     lock/write  │
                                  ├── git2: inspection, staging facts and local ref operations
                                  └── system Git: add/commit/unstage, merge/pull/push,
                                                  clone/fetch and passthrough
                                           │
                                      report + table + color
```

## Module responsibilities

- The `cli` facade keeps `crate::cli::*` stable. `invocation` separates native Git passthrough at an explicit `--`; `command` defines the clap top-level surface; `args` stores argument models by automation, workspace, synchronization, branch, inspection, execution, and schedule domains.
- The `automation` facade keeps protocol call paths stable. `options` validates output format, request ID, timeout, and plan/apply options. `output` only serializes v1 JSON envelopes, JSONL lifecycles, and workspace revision context. `error` centralizes stable error classification and URL user-info sanitization.
- `settings` resolves CLI, environment, and default-value precedence. `env list` reuses that path to show effective values instead of maintaining a second runtime configuration.
- `commands/mod.rs` retains only top-level dispatch, plan/apply revision verification, shared child-process policy, and batch progress. `plan`, `automation_commands`, `remote`, `changes`, `branches`, and `exec` own their respective workflows. `workspace_commands` is split further into clone, scan, restore, manifest membership, and shared path/naming invariants. `inspect` is a facade; list, status, find, info, and branch own their query models and failure semantics. A single-repository failure becomes an aggregate result and must not cancel other repositories.
- `workspace` discovers roots, owns exclusive locking, and atomically replaces `batchspace.toml`.
- `model` defines schema version 1, cross-field validation, and serializable models.
- The `git` facade keeps call paths stable. `types` contains value objects; `execution` centralizes system Git environment isolation, interaction, and timeouts; `clone`, `checkout`, `inspect`, `remotes`, and `discovery` own cloning, branch switching, read-only facts, remote configuration, and working-tree discovery. The command layer still invokes system Git for add/commit/unstage, merge/pull/push, and similar operations through the shared execution policy.
- The `schedule::commands` facade creates one lightweight `CommandContext` per invocation and centralizes concurrency, output, and revision policy. `declarations`, `execution`, `query`, `native`, and `support` handle manifest declarations, planning/running, read-only queries, native task lifecycle, and pure lookup/rendering rules. `artifact` explicitly generates launchd, systemd user-timer, and Windows Task Scheduler definitions. `registration` isolates native system calls, while `state` stores and validates local registration summaries. Host paths never enter the shared manifest.
- `parallel` enforces concurrency limits and input-order collection. `report` separates business results, lifecycle events, protocol serialization, text summaries, and child-output blocks into `result`, `jsonl`, `machine`, `text`, and `child_output`. `table` provides stable alignment.

The default Cargo feature, `schedule`, compiles the complete schedule command and native
integration. Without default features, both the CLI and `capabilities.commands` omit schedule, but
`model` still parses and preserves schedule declarations. launchd, systemd, and Windows artifacts
remain cross-platform generation targets; only real host operations use `target_os` conditional
compilation.

`cli::metadata` is the compile-time source of truth for canonical command names, compatibility
aliases, mutability, global plan support, and capabilities exposure. `Command::kind()`, dispatch,
and plan remain exhaustive matches, so adding an enum variant forces the compiler to identify
missing handling without introducing a dynamic registry.

The `--output json` boundary lives in `automation`: a successful invocation emits one receipt, and
the library entry point converts errors into structured documents. `report` maps batch
`RepositoryResult` values into stable per-repository records without placing child Git stdout/stderr
in the protocol. `--output jsonl` emits lifecycle and repository-terminal events from the same data
model. `clone` models its single managed Git operation as one repository event instead of defining a
separate progress protocol.

Top-level `error.code` is read only from typed classifications in the error chain; `anyhow` continues
to carry context. Natural-language changes or incidental words such as `lock`, `schedule`, or
`timeout` in lower-level output cannot change the machine classification.

## Consistency and side effects

Every workflow that may run Git or write the manifest first uses the hidden workspace `.batchspace.lock`, then the legacy `.workspace.lock` in the same fixed order. The compatibility lock prevents upgraded and pre-rename clients from splitting coordination during v1.
Manifest writes go to a same-directory temporary file, are synchronized, and then atomically
replace the destination. Batch commands are not transactions across repositories: each repository
completes or fails independently, and the final report preserves the distinction between success,
skipped, and failed.

`sync` is the unattended default: it restores missing repositories and fetches/prunes remote
references without changing existing working trees. `pull` is always fast-forward-only. `add`
stages all non-ignored additions, modifications, and deletions and rejects unresolved conflicts.
`commit` commits the existing index only, rejects detached HEAD and in-progress Git operations, and
does not provide implicit add, amend, empty commits, or hook bypass. `unstage` resets the entire index
without changing the working tree; an unborn HEAD uses `git read-tree --empty`. History rewriting,
working-tree cleanup, and force-push remain outside the built-in automation surface.

`merge` uses existing local references by default. Explicit `--update-current` / `--uc` first runs a
fast-forward-only pull when the current target branch has an upstream; local-only branches skip that
step. Explicit `--refresh-source` / `--rs` fetches declared remotes and merges the newest
remote-tracking source without moving the local source branch. Both can modify the working tree.
Conflicts are left for the user: batch-git never aborts, continues, rebases, or rolls back
automatically.

`merge --default` enables source refresh by default and can be disabled with
`BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE`. `merge --feature` does not update the current target by
default and can be enabled with `BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT`. These source-specific
defaults apply only to their respective modes; explicit CLI options always win.

Commit obeys repository hooks, identity, and signing configuration, which may have local or external
side effects that batch-git cannot classify. Per-repository Git index/ref locks and the workspace
lock reduce concurrency conflicts, but native Git invoked outside batch-git does not honor
`.batchspace.lock` and the v1 compatibility `.workspace.lock`. Batch commands are non-transactional: if later repositories fail or time out,
commits already created in earlier repositories are not reset, amended, rebased, or otherwise rolled
back. After the direct Git child times out, hook, filter, signing, authentication, or transport
descendants may still be alive; callers must recheck HEAD, the index, and the working tree.

A plan is not a transaction. `--plan` reads local state and returns the `batchspace.toml` digest;
`--apply` rechecks that digest after acquiring the lock and only then runs the write operation.
Add/commit/unstage plans also expose fixed index scope, commit message, or working-tree preservation
properties, but they do not freeze HEAD, the index, or the working tree. No additional ledger is
stored, remote state is not locked, and cross-repository rollback is not promised.

System Git uses `GitExecutionOptions` to centralize stdin, `GIT_TERMINAL_PROMPT`, and per-child
timeouts. Machine output forces non-interactive execution so child streams cannot corrupt JSON.
Reader threads always drain stdout/stderr, but retain only a 1 MiB head/tail diagnostic window per
stream to prevent concurrent `log`, `diff`, or error output from causing unbounded memory growth.

Manifest directories first pass lexical relative-path validation and then
`workspace::repository_path` filesystem-boundary resolution. Existing paths and the nearest existing
ancestor must canonicalize inside the workspace root. Symlinks that remain inside the workspace are
allowed; repositories or prospective child paths that resolve outside it are rejected when the
manifest is read and before each repository operation.

## Evolution rules

- `batchspace.toml` schema changes are guarded by `version`; breaking format changes require a migration and a new major version.
- v1 JSON/JSONL output and public schemas are automation contracts. Only new optional fields may ship within the same major version.
- A new scheduler platform must implement generation, validation, registration, status, and safe unregistration, with CI coverage on the target platform.
- New commands must define selector, concurrency, exit-code, partial-failure, and documentation behavior, not only a happy path.
- Future help and metadata work follows the [command organization plan](COMMAND_ORGANIZATION_PLAN.md). Do not introduce nested command paths or change existing command strings until that plan explicitly enters implementation.
