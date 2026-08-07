# batch-git

[![CI](https://github.com/ovaso/batch-git-rs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ovaso/batch-git-rs/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/ovaso/batch-git-rs?sort=semver&display_name=tag)](https://github.com/ovaso/batch-git-rs/releases/latest)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[简体中文](README.zh-CN.md)

`batch-git` is a multi-repository Git workspace manager. It uses one portable, reviewable
`workspace.toml` manifest to manage multiple independent Git repositories; the workspace itself
does not need to be a Git repository.

## Highlights

- Scan existing directories and create a workspace manifest.
- Restore missing repositories from the manifest, or clone and register one repository.
- Run add, commit, unstage, fetch, fast-forward pull, push, checkout, merge, and status operations in batches.
- Select repositories explicitly or by name pattern before running native Git commands.
- Register scheduled synchronization with macOS `launchd`, Linux `systemd --user`, or Windows Task Scheduler.
- Use bounded concurrency, deterministic output order, workspace locking, and aggregate exit codes.
- Integrate with CI and agents through versioned JSON/JSON Lines protocols, structured errors, capability discovery, and manifest preconditions for plan/apply.

`batch-git commit` commits only content that is already staged. It never performs an implicit add,
amend, empty commit, or hook bypass. The tool also never merges, rebases, stashes, resets, or cleans
a working tree unless explicitly requested. If clone or restore fails or times out, the destination
is preserved for inspection and must be handled explicitly before retrying.

## Distribution and trust

Generated code is cheap; maintenance commitments are not.

Accordingly, this project does not use public package registries to make quality, maintenance, or
security commitments that it cannot sustain over the long term.

This project may use AI-assisted development, but it does not treat "it runs" as evidence that
software is ready for distribution. Trust comes from explicit safety boundaries, tests, review,
documentation, and traceable release processes, not from how the code was produced.

`batch-git` is intentionally not published to crates.io or other package registries and does not
claim public package names it cannot continuously maintain. This GitHub repository and its Releases
are the only authoritative distribution sources. Same-named packages elsewhere are outside this
project's release process and trust boundary.

## Installation

Tagged GitHub Releases may contain prebuilt archives, SHA-256 checksums, and GitHub artifact
attestations for Linux x86_64, macOS x86_64/arm64, and Windows x86_64. The included installers never
use `sudo` and require an explicit version:

```sh
# macOS / Linux: download and review the installer before installing to ~/.local.
VERSION=vX.Y.Z
curl -LO "https://github.com/ovaso/batch-git-rs/releases/download/$VERSION/install.sh"
sh install.sh --version "$VERSION"

# Optional: install to another user-writable prefix.
sh install.sh --version "$VERSION" --prefix "$HOME/.local"
```

PowerShell:

```powershell
$Version = "vX.Y.Z"
Invoke-WebRequest "https://github.com/ovaso/batch-git-rs/releases/download/$Version/install.ps1" -OutFile install.ps1
.\install.ps1 -Version $Version
```

The installers verify the SHA-256 file published beside each archive. You can also download and
verify an archive manually from [GitHub Releases](https://github.com/ovaso/batch-git-rs/releases).
If GitHub CLI is available, verify the signed provenance as well:

```sh
gh attestation verify batch-git-<target>.tar.gz --repo ovaso/batch-git-rs
```

To build from source, use a checked-out source tree with a Rust toolchain and system Git:

```sh
# Install the current checkout.
cargo install --locked --path .

# Build only.
cargo build --release
./target/release/batch-git --version

# Optional: build without the schedule command and native scheduler integration.
cargo build --release --no-default-features

# Build and install to a selected directory.
BATCH_GIT_INSTALL_PATH="$HOME/.local/bin" ./build.sh
```

Current release archives include Bash, Zsh, Fish, and PowerShell completions. The installer copies
the completion files for the current platform into the selected user prefix. If your shell does not
discover that prefix automatically, follow the
[user guide](docs/USER_GUIDE.md#2-installation-and-verification).

The default feature set includes `schedule`, so normal builds retain the complete command surface.
Disabling default features only removes the `schedule` command and native scheduler integration
from that binary. Existing schedule declarations are still parsed and preserved in `workspace.toml`,
preventing a minimal build from discarding manifest data. Automation should inspect
`capabilities.data.commands` before attempting schedule operations.

Make sure the installation directory is on `PATH`:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

The installed binary is named `batch-git`. `bit` is only an optional user-defined shell alias; see
the [user guide](docs/USER_GUIDE.md#2-installation-and-verification).

## Quick start

Create a workspace from an existing directory containing multiple repositories:

```sh
cd /path/to/workspace
batch-git scan
batch-git status
batch-git branch
batch-git fetch

# Stage explicitly, review, then commit. Omitting selectors targets the whole workspace.
batch-git add --match 'service-*'
batch-git exec --match 'service-*' -- diff --cached --stat
batch-git commit --match 'service-*' -m 'Update generated clients'
```

Restore a workspace from its manifest:

```sh
mkdir restored-workspace
cp workspace.toml restored-workspace/
cd restored-workspace
batch-git restore
batch-git fetch
```

Run native Git across repositories:

```sh
# Every materialized repository.
batch-git -- status --short

# Selected repositories.
batch-git exec service-api service-web -- log -1 --oneline

# Repositories matched by name.
batch-git exec --match 'service-*' -- fetch --prune
```

The command boundary is explicit:

```text
batch-git <command> [options]       # built-in batch-git command
batch-git -- <git-args...>          # pass arguments unchanged to Git in every repository
```

Unknown built-in commands fail instead of being interpreted as Git commands. For example,
`batch-git branch` displays a workspace branch summary, while `batch-git -- branch` runs
`git branch` in every repository.

## Common commands

| Workflow | Command |
|---|---|
| Create or extend the manifest | `batch-git scan` |
| Inspect workspace status | `batch-git status` |
| Show current branches | `batch-git branch` |
| Show supported environment variables and effective values | `batch-git env ls` |
| Stage all non-ignored changes | `batch-git add [repositories]` |
| Commit staged content | `batch-git commit [repositories] -m <message>` |
| Unstage everything while preserving working trees | `batch-git unstage [repositories]` |
| Safely update remote references | `batch-git fetch` or `batch-git sync` |
| Fast-forward the current branch | `batch-git pull` |
| Push the current tracking branch | `batch-git push` |
| Check out the same branch | `batch-git checkout <branch>` |
| Check out each repository's default branch | `batch-git cd` |
| Merge each repository's default branch into the current branch | `batch-git merge --default` |
| Search local or remote branches | `batch-git find 'feature/*'` |
| Show workspace or repository details | `batch-git info [repository]` |
| Manage scheduled jobs | `batch-git schedule --help` |

Use `batch-git --help` and each subcommand's `--help` output as the complete parameter reference.

## Automation and agents

New automation should use global `--output json` instead of parsing tables or legacy subcommand
`--json` output:

```sh
# Discover the current binary's capabilities before consuming stable receipts.
batch-git --output json capabilities
batch-git --output json --request-id ci-184 status

# Preview a batch sync; apply rechecks the workspace revision returned by the plan.
batch-git --output json --plan sync --match 'service-*'
batch-git --output json --apply --expect-workspace-revision 'sha256:…' \
  sync --match 'service-*'
```

For long-running operations, `--output jsonl` emits `started`, `repository_finished`, and `finished`
events one line at a time. A single-repository clone uses the same lifecycle. Repository events stay
in manifest order, so a later repository that finishes first may wait for earlier events.
`--non-interactive` disables Git prompts. `--timeout 5m` limits only the directly launched Git child
process and cannot guarantee termination of authentication or transport descendants.

The schema returned by `schema workspace` describes the JSON representation of `workspace.toml`;
fields with defaults may be omitted from the manifest. See the
[automation contracts](docs/AUTOMATION_CONTRACTS.md) for complete fields, compatibility rules, and
safety boundaries. The repository-local skill provides the default workflow for agents.

## Documentation

- [User guide](docs/USER_GUIDE.md): installation, workflows, commands, and troubleshooting.
- [Workspace manifest](docs/WORKSPACE.md): `workspace.toml`, environment variables, and state directories.
- [Schedules](docs/SCHEDULES.md): declarations, validation, registration, logs, and platform differences.
- [Development and release](docs/DEVELOPMENT.md): validation commands, implementation boundaries, and release checks.
- [Architecture](docs/ARCHITECTURE.md): module responsibilities, concurrency, locking, and Git execution boundaries.
- [Compatibility](docs/COMPATIBILITY.md): validated Rust, Git, platform, and scheduler matrix.
- [Automation contracts](docs/AUTOMATION_CONTRACTS.md): versioned JSON/JSONL, plan/apply, schemas, and exit codes for CI and agents.
- [Changelog](CHANGELOG.md): release-level changes and known limitations.

The authoritative behavior is defined by command help, these guides, and the current code.

## Exit codes

- `0`: the command completed; expected skips such as nothing to stage, commit, or unstage are allowed.
- `1`: at least one repository operation failed.
- `2`: argument, environment, workspace, manifest-validation, or file-write error.

## Development checks

```sh
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --locked --release
```

## Contributing

Before contributing, read the [contributing guide](CONTRIBUTING.md), [security policy](SECURITY.md),
and [agent collaboration guidelines](AGENTS.md). Automation for multi-repository Git operations can
use the repository-local, uninstalled
[batch-git automation skill](skills/batch-git-automation/SKILL.md).

## License

This project is licensed under the [MIT License](LICENSE).
