# Changelog

[简体中文](CHANGELOG.zh-CN.md)

This project follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
[Semantic Versioning](https://semver.org/). See `Cargo.toml` for the current package version.

## [Unreleased]

### Changed

- Documented the repository-only distribution and trust policy: generated code is not treated as
  release readiness, public package registries are intentionally excluded, and this GitHub
  repository plus its Releases define the project's distribution trust boundary. The Cargo manifest
  now disables registry publication.
- Replaced fixed release-support, response-time, disclosure, and backport promises with statements
  of current CI validation scope and best-effort maintainer availability.

## [0.4.5] - 2026-08-07

### Fixed

- Corrected repository metadata, installer downloads, and release and attestation links to use `ovaso/batch-git-rs`, while preserving the legacy v1 automation schema identifiers for compatibility. Release badges now select the latest semantic-version tag, and installation documentation no longer advertises crates.io installation.

## [0.4.4] - 2026-08-07

### Added

- Release archives now include `LICENSE`, English and Simplified Chinese READMEs, and Bash/Zsh/Fish/PowerShell completions. New macOS/Linux and Windows user-directory installers require an explicit version, never use sudo, and verify SHA-256. The release workflow signs final archives with GitHub build provenance attestations and publishes cargo-binstall release metadata.
- Public project documentation is now English-first with complete Simplified Chinese mirrors, cross-language navigation, and bilingual GitHub contribution templates.

### Changed

- GitHub Actions now uses supported macOS 15 runner labels, cancels superseded CI runs on the same branch or pull request, validates remote release tags before starting platform builds, and serializes releases for the same tag.
- Raised the documented and CI-tested minimum supported Rust version from 1.85 to 1.88, matching the language features used by the current source.
- Split command orchestration into dedicated modules for planning, automation discovery, workspace lifecycle, read-only inspection, synchronization/remotes, staging/commits, branches, and Git passthrough. Workspace lifecycle is further separated into clone, scan, restore, and manifest membership; read-only inspection is separated into list, status, find, info, and branch. Schedule commands are split into declarations, execution, queries, native lifecycle, and pure support rules while retaining isolated three-platform artifacts, registration system calls, and registration state. CLI paths, exit codes, JSON/JSONL fields, capability command strings, and schedule behavior are unchanged.
- Split Git execution, clone, checkout, inspection, remotes, discovery, and result types; CLI invocation/command/domain arguments; report result/JSONL/machine/text output; and automation options/protocol output/error classification behind facades. Added a default-enabled `schedule` Cargo feature: normal builds keep the full command surface, while minimal builds can conditionally omit schedule commands and native integration.
- Centralized canonical command names, compatibility aliases, mutability, global plan support, and capabilities exposure in the compile-time `cli::metadata` table. Dispatch and plan remain exhaustive matches; CLI paths, command strings, and public semantics are unchanged.
- Git child stdout/stderr is continuously drained but retains at most a 1 MiB head/tail window per stream. A stable truncation marker is inserted beyond the limit, preventing unbounded memory growth from concurrent large output.

### Fixed

- Manifest repository paths now validate the canonical path of an existing location or its nearest existing ancestor in addition to lexical checks. Symlinks that remain inside the workspace are allowed; symlinks resolving an existing repository or clone/restore destination outside the workspace are rejected.
- Top-level machine error codes now come from typed `ErrorCode` values instead of matching natural-language strings such as `lock`, `schedule`, or `timed out`, preventing contextual wording from changing classifications.

### Validation

- CI and Release platform builds now record Linux and macOS dynamic dependency inspection, and the contribution guide documents the protected-main pull-request and post-merge tagging workflow.
- Added regression coverage for the flat top-level command surface and complete capabilities roster, preventing later internal work from accidentally renaming commands or adding nested paths.
- Added compilation, CLI-surface, and capability-roster checks without default features. Default features continue to run complete schedule, automation-protocol, and MVP regressions.
- CI now runs no-default-feature check, full tests, Clippy, release build, and behavioral verification for minimal help, capabilities, and the absent schedule command.
- Added black-box coverage for workspace-escaping symlinks, unit tests for typed error classification, and bounded-retention tests for multi-MiB child output.

## [0.4.3] - 2026-08-06

### Added

- Added read-only `batch-git env list` (alias `env ls`) to show supported environment variables, defaults, and effective values for the current invocation. `-d` / `--description` adds descriptions; text mode displays `CURRENT` in green on compatible terminals and supports v1 JSON/JSONL plus capability discovery.

### Changed

- `BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE` is enabled by default when merging default branches, refreshing and merging the newest remote default. `BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT` is disabled by default for `merge --feature` and updates the current branch only when configured. `--uc`, `--rs`, and their `--no-*` forms always take precedence.
- Removed the source-agnostic `BATCH_GIT_MERGE_UPDATE_CURRENT` and `BATCH_GIT_MERGE_REFRESH_SOURCE` environment variables.

### Fixed

- Text tables now soft-wrap overlong cells to available terminal width while keeping continuation lines aligned to the original column instead of falling back to column one. ANSI color and Unicode width calculations remain correct after wrapping.

## [0.4.2] - 2026-08-06

### Added

- Added `merge --refresh-source` (`--rs`): after refreshing source remote references, merge the newest remote-tracking source without moving the local source branch. It can be combined with the new `--uc` alias for `--update-current` to fast-forward the target first. Both behaviors were configurable through `BATCH_GIT_MERGE_UPDATE_CURRENT` / `BATCH_GIT_MERGE_REFRESH_SOURCE` defaults and overridable by CLI enable or `--no-*` options.

### Fixed

- `merge --update-current` (`--uc`) now skips the fast-forward-only pull when the target branch has no upstream, allowing local-only feature branches to continue merging the source.

## [0.4.1] - 2026-08-06

### Fixed

- Without `--update-current`, `merge` now rejects a current branch whose local tracking ref already shows it is behind or diverged. This avoids starting a Git merge on an outdated target and leaving an in-progress merge that blocks a later pull.

## [0.4.0] - 2026-08-04

### Added

- Added safe batch `batch-git add`, reusing name, relative-directory, `--match`, and `--all` selectors and targeting the whole workspace when omitted. The first version accepts no pathspec, stages all non-ignored additions/modifications/deletions, and rejects unresolved conflicts.
- Added `batch-git commit -m <message>`, which commits only the existing index, never implicitly adds, and offers no amend, empty-commit, or hook-bypass behavior. It rejects detached HEAD, unresolved conflicts, and in-progress merge/rebase/cherry-pick/revert operations.
- Added full `batch-git unstage`, which restores only selected indexes, preserves working trees, and does not move HEAD. An unborn HEAD safely clears the index with `git read-tree --empty`.
- Integrated add/commit/unstage with v1 JSON/JSONL, plan/apply, capabilities, and stable reason codes. Plans expose `git_indexes`, `git_objects`, `local_refs`, `hooks`, and command-parameter boundaries.

### Fixed

- `unstage` now uses structured HEAD state to distinguish a truly unborn repository from a valid branch literally named `(unborn)`, preventing an incorrect full-index clear in the latter case.
- When an in-progress Git operation also has unresolved conflicts, `commit` consistently returns `unresolved_conflicts`; resolved but incomplete merge/rebase/cherry-pick/revert operations continue to return `repository_operation_in_progress`.

### Security

- Commit preserves repository hooks, identity, and signing policy and offers no automatic bypass. Machine mode remains non-interactive. Hook, filter, signing timeout, or cross-repository failure never triggers automatic reset, amend, or rebase; callers must inspect partial success and possibly updated local refs.

## [0.3.0] - 2026-08-04

### Added

- Added `batch-git merge --default` (short option `-d`), reading each repository's `default_branch` from `workspace.toml` and merging it into the current branch. Multi-component names are supported; when no local branch exists, resolution falls back only to that repository's `primary_remote` and does not fetch implicitly.
- Merge plans now expose `source_branch`, `source_mode`, and `remote_fallback` per repository so explicit, feature-environment, or per-repository default sources can be reviewed before apply.

### Fixed

- Ordinary `merge` now correctly reads the documented `BATCH_GIT_REMOTE`, resolving same-named source branches across multiple remotes through environment configuration.

### Security

- Upgraded `git2` to `0.21.0`, fixing potential undefined behavior reported by RUSTSEC-2026-0183 and RUSTSEC-2026-0184. Network operations remain delegated to system Git, and default git2 network features remain disabled.

### Validation

- Added CLI and automation-protocol black-box regressions for different complex default branches, primary-remote fallback, multi-remote ambiguity, argument conflicts, and side-effect-free plans. cargo-deny advisories, bans, licenses, and sources checks pass.

## [0.2.0] - 2026-07-31

### Added

- Added GitHub Actions quality gates, three-platform release builds, dependency upgrades, and security audits.
- Added contribution, security, compatibility, architecture, automation-contract, and agent-collaboration documentation.
- Added the repository-local `batch-git-automation` skill for safely orchestrating multi-repository operations.
- Added automation protocol v1: global `--output json|jsonl`, `--request-id`, machine receipts for every public built-in and schedule subcommand, and manifest-ordered batch repository event streams that do not contaminate stdout. Single-repository clone also emits a complete `started`, `repository_finished`, `finished` lifecycle.
- Added `capabilities` and `schema operation-result|workspace`, allowing agents to discover protocol, safety boundaries, and JSON Schema from the current binary instead of guessing the installed version.
- Added `--plan` / `--apply --expect-workspace-revision`: side-effect-free previews of resolved scope, risk, and side effects, with `workspace.toml` SHA-256 revision verification before execution.
- Added `--non-interactive` and per-system-Git-child `--timeout`. Timeout terminates only the direct child and Git descendants may survive. Clone/restore destinations are preserved after timeout or failure to avoid recursively deleting concurrently written content.
- The hidden native-schedule `native-run` child is always non-interactive, preventing a scheduler or terminal-less environment from waiting for Git authentication prompts. A stale revision found after the child acquires the lock remains `stale_workspace_revision` rather than becoming a generic scheduler error.
- Added automation-protocol black-box tests for legacy/new JSON compatibility, structured argument errors, schedule receipts, and stale-plan rejection.

### Changed

- `status` and `branch` gained compatible direct `--json` payloads. Existing `list`, `find`, `info`, and schedule `--json` top-level shapes remain unchanged.
- Machine mode no longer forwards raw Git or native-scheduler stdout/stderr. Batch results use stable status and `reason_code`, preventing credentials or unstructured diagnostics from entering the protocol.

## [0.1.1] - 2026-07-29

Patch release fixing Git proxy, remote configuration, and schedule concurrency while reducing artifact size.

### Fixed

- Clone now uses system Git while retaining branch selection, shallow clone, single-branch mode, custom remote names, Git credential configuration, and interactive authentication. Shallow clones of local paths retain true shallow semantics.
- Fixed stale local push URLs not being cleared when the manifest omitted a separate `push_url`, which could send later pushes to an address not declared by the manifest.
- Fixed Git passthrough stdout/stderr remaining piped under `--jobs 1`, which broke `rebase -i`, editors, and other TTY-interactive commands.
- Schedule register/unregister now participates in the workspace lock, preventing stale-configuration overwrites and registration-state races with update/remove or concurrent registration.

### Build and release

- Disabled git2/libgit2 network features; all network access is delegated to system Git.
- Removed libssh2 and OpenSSL runtime dependencies; macOS artifacts no longer depend on a Homebrew OpenSSL path.
- Removed Chrono's unused Serde feature.
- Enabled size optimization, LTO, one codegen unit, abort-on-panic, and symbol stripping for release. The reference macOS arm64 artifact dropped from approximately `4.5 MiB` to `1.9 MiB`.

### Validation

- Added regressions for system-Git clone argument proxying and push-URL synchronization.
- All unit/end-to-end tests, Clippy, formatting, and release builds passed.

## [0.1.0] - 2026-07-29

First stable release.

### Workspace management

- Scan existing repositories, clone and register, restore from the manifest, fetch/prune, and safe sync.
- Store repositories, remotes, default branches, and schedule declarations in `workspace.toml`.
- Validate manifests, write atomically, sanitize URL credentials, and lock the workspace.
- Inspect workspace/repository information, status, current branches, lists, and JSON output.

### Branch and Git operations

- Search local/remote branches, check out default and feature branches, create branches from explicit starting points, and merge the current feature branch directly.
- Resolve same-named remote branch ambiguity.
- Optionally fast-forward before merge, plus an independent safe pull.
- Safe batch push, explicit upstream creation on first push, whole-workspace Git passthrough, and repository/pattern-selected `exec`.
- Aggregate `git commit` “nothing to commit” as skipped.

### Execution experience

- Bounded concurrency, manifest-order output, and per-repository aggregate results.
- Interactive progress, stable non-interactive tables, Unicode width alignment, and `NO_COLOR`.
- Exit codes distinguish completion, repository-level failure, and configuration/validation errors.

### Scheduled jobs

- `sync` / `pull`, daily time, fixed intervals, and six-field cron.
- Plan, validate, generate, register, update, status, immediate run, unregister, and remove.
- macOS launchd, Linux systemd user timers, and Windows Task Scheduler.
- Overlap policy, registered-definition consistency checks, and optional background logs.

### Known limitations

- Manifest rewrites do not preserve TOML comments or original formatting.
- Schedule timezone uses `BATCH_GIT_TZ`; when unset, the system timezone applies.
- The launchd cron seconds field must be `0`.
- Windows Task Scheduler does not yet support cron; fixed intervals range from `1m` through `31d`.
- Quartz `L`, `W`, `#`, and year fields are unsupported.
- Batch operations provide no cross-repository transactional rollback and may partially succeed.
- Interactive Git children require `--jobs 1`.

[Unreleased]: https://github.com/ovaso/batch-git-rs/compare/v0.4.5...HEAD
[0.4.5]: https://github.com/ovaso/batch-git-rs/compare/v0.4.4...v0.4.5
[0.4.4]: https://github.com/ovaso/batch-git-rs/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/ovaso/batch-git-rs/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/ovaso/batch-git-rs/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/ovaso/batch-git-rs/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/ovaso/batch-git-rs/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/ovaso/batch-git-rs/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/ovaso/batch-git-rs/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/ovaso/batch-git-rs/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/ovaso/batch-git-rs/releases/tag/v0.1.0
