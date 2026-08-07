# Development and Release Guide

[简体中文](zh-CN/DEVELOPMENT.md)

## 1. Technical boundaries

- `git2/libgit2`: local repository inspection, checkout, remote configuration, and branch configuration; network features are disabled.
- System Git CLI: add, commit, restore/read-tree unstage, clone, fetch, push, merge, pull, and exact passthrough after `--`.
- `batchspace.toml`: declarative, portable workspace manifest.
- `.batchspace.lock`: hidden canonical runtime coordination file that serializes batch-git processes that may modify Git state or the manifest. It remains after a process exits; the operating-system lock is released with the process file handle. v1 also acquires the legacy `.workspace.lock` in a fixed order so upgraded clients remain mutually exclusive with older clients.
- `automation result v1`: stable protocol for global `--output json|jsonl`; legacy subcommand `--json` is compatibility-only.
- Rayon: bounded concurrency and stable result order.
- clap: strict built-in command boundaries and command help.

Implicit actions that change commit relationships or lose working-tree data are forbidden. Unless the
user explicitly invokes the corresponding command, do not automatically pull, merge, rebase, stash,
reset, clean, or switch branches. Built-in `commit` reads the existing index only and must not gain
implicit add, amend, empty-commit, or hook-bypass behavior; it rejects detached HEAD, unresolved
conflicts, and in-progress Git operations. The first version of built-in `add` always stages every
non-ignored addition, modification, and deletion and rejects conflicts. `unstage` changes only the
index, preserves the working tree, and uses `git read-tree --empty` for an unborn HEAD. Do not add a
second persistent state source.

## 2. Local validation

Run the following before committing or releasing:

```sh
cargo fmt -- --check
cargo test --locked
cargo check --locked --no-default-features
cargo test --locked --no-default-features
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo build --locked --release
cargo build --locked --release --no-default-features
./target/release/batch-git --help
./target/release/batch-git add --help
./target/release/batch-git commit --help
./target/release/batch-git unstage --help
./target/release/batch-git env --help
./target/release/batch-git schedule --help
```

The default `schedule` feature preserves the command surface used by GitHub Release builds.
`--no-default-features` validates the minimal build. That build must continue to read and preserve
schedule declarations, but neither `capabilities.commands` nor top-level help may claim the
uncompiled `schedule` command exists. With the feature enabled, all three platform artifact
generators participate in compilation and tests to preserve cross-platform previews. Only real host
system calls may use `target_os` conditional compilation.

Before release, also run `cargo deny check advisories bans licenses sources`. GitHub Actions applies
the complete quality gate on Linux with both default and no-default features and builds release
artifacts on Linux, macOS, and Windows. It also publishes a downloadable LCOV coverage report. A
`v*` tag triggers GitHub Release archives containing completions and the license, per-file and
combined SHA-256 checksums, installers, and GitHub build provenance attestations.

The protected `main` branch accepts changes through pull requests only. Version and changelog
updates must merge and pass the required CI checks before the release tag is created. The Release
workflow validates that its `v*` tag already exists in the remote repository before starting the
platform matrix; manual dispatch is only for retrying an existing tag. Runs for the same tag are
serialized so concurrent dispatches cannot race while publishing one GitHub Release.

Release builds must also inspect dynamic dependencies to ensure nothing links unexpectedly to
Homebrew, another package manager, or a non-system absolute path from the build host. Use
`otool -L target/release/batch-git` on macOS or `ldd target/release/batch-git` on Linux. The CI and
Release platform matrices record these inspections in their job logs.

`tests/mvp.rs` covers the primary end-to-end workflows. Changes to add/commit/unstage should cover
the index, working tree, unborn branches, conflicts, detached HEAD, hooks, and partial success.
Module unit tests cover parsing, manifest validation, output, and platform definition generation.
`tests/automation_protocol.rs` covers v1 receipts, legacy JSON compatibility, structured argument
errors, JSONL lifecycles, staging-command reason codes/capabilities/plan parameters, and revision
preconditions both before and after plan/apply locking.

## 3. Documentation maintenance

- `README.md` is the English project entry point; `README.zh-CN.md` is its Simplified Chinese mirror. Keep positioning, installation, quick start, and document links there.
- English topic guides live in `docs/`; synchronized Simplified Chinese translations live in `docs/zh-CN/`.
- User behavior belongs in `docs/USER_GUIDE.md`.
- Manifest and environment variables belong in `docs/WORKSPACE.md`.
- Scheduler platform details belong in `docs/SCHEDULES.md`.
- Release-level delivery belongs in `CHANGELOG.md`, with `CHANGELOG.zh-CN.md` kept in sync.
- Machine-readable output changes must update `AUTOMATION_CONTRACTS.md`. New fields may be backward-compatible; removals or type changes require the next major version.
- Changes to `--output`, `--request-id`, `--non-interactive`, `--timeout`, `--plan` / `--apply`, reason codes, JSONL events, or `schema` must update the automation contracts, user guide, capability roster, repository-local skill, and black-box protocol tests.
- Public behavior or safety-boundary changes must enter the `Unreleased` section of both changelogs.
- Do not rely only on comments for cross-module decisions. Update `ARCHITECTURE.md` when persistent formats, locking, concurrency, or the system Git boundary changes.

When adding or changing CLI arguments, check together:

1. clap `--help` text.
2. The README common-command table.
3. The relevant topic guide in both languages.
4. Environment-variable tables and manifest examples.
5. Both changelogs.

## 4. Release checklist

- [ ] The version in `Cargo.toml` matches `CHANGELOG.md`.
- [ ] Formatting, tests, Clippy, and the release build pass.
- [ ] `batch-git --help` matches the user guide.
- [ ] The `batchspace.toml` example is accepted by the current schema.
- [ ] macOS/Linux/Windows schedule constraints are documented.
- [ ] Documentation does not describe `bit` as an automatically installed command.
- [ ] Known limitations are recorded.
- [ ] The release artifact reports the correct `batch-git --version`.
- [ ] Dynamic dependencies contain no private build-host or package-manager absolute paths.
- [ ] `cargo deny check advisories bans licenses sources` passes.
- [ ] Linux, macOS, and Windows release builds pass in CI.
- [ ] The release commit reached `main` through a pull request and all required checks passed.
- [ ] The remote `v*` tag points at that exact merged release commit.
- [ ] The tag, GitHub Release, binary names, and SHA-256 checksums agree.
- [ ] `gh attestation verify <archive> --repo ovaso/batch-git-rs` verifies the release archive.
- [ ] `sh -n install.sh`, the PowerShell parser, and all four completion checks pass.
- [ ] English and Simplified Chinese documentation links are valid and translations are synchronized.
- [ ] `SECURITY.md`, `COMPATIBILITY.md`, and the automation contracts still match behavior.

## 5. Build and installation scripts

```sh
BATCH_GIT_INSTALL_PATH="$HOME/.local/bin" ./build.sh
```

The script performs a release build, creates the installation directory when necessary, and
installs the executable as `$BATCH_GIT_INSTALL_PATH/batch-git`. It fails when the installation path
is unset or exists but is not a directory.

`install.sh` and `install.ps1` install published prebuilt archives. They require an explicit tag,
download the archive and matching `.sha256`, install the binary and completions into a user-writable
prefix, and never request administrator privileges. The release workflow packages `LICENSE`,
`README.md`, `README.zh-CN.md`, and `completions/` and attests the final archive. The installers verify SHA-256 only;
users or CI perform stronger provenance verification separately with GitHub CLI.
