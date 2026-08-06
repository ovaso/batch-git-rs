# Compatibility

[简体中文](zh-CN/COMPATIBILITY.md)

## Toolchain

| Component | Supported baseline | Notes |
|---|---:|---|
| Rust | 1.88 | Minimum toolchain required by the current source; CI tests both this version and stable. |
| Git | 2.30+ | System Git is required for add, commit, restore/read-tree unstage, clone, fetch, pull, push, merge, and passthrough. Use a maintained current release when possible. |
| workspace.toml | version 1 | The only currently supported manifest schema. |

## Platforms

| Platform | CLI | Schedule backend | CI build |
|---|---|---|---|
| Linux x86_64 | Supported | `systemd --user` | Release archive |
| macOS arm64/x86_64 | Supported | `launchd` | Release archive |
| Windows x86_64 | Supported | Task Scheduler | Release archive |

CI builds release binaries on all three operating systems. Archives include the license, both READMEs,
and Bash/Zsh/Fish/PowerShell completions, and publish per-file and combined SHA-256 checksums plus
GitHub build provenance attestations. Real registration still depends on runner user permissions and
platform service availability. Before release, run `schedule doctor` and `generate` on each target
platform and manually verify register/unregister where needed.

## Known platform constraints

- The seconds field in launchd cron expressions must be `0`.
- Windows does not support cron; `--every` accepts only `1m` through `31d`.
- Linux schedules require an available user-level systemd session.
- The operating system determines the default timezone. Re-register after setting `BATCH_GIT_TZ`.
- Interactive Git children require `--jobs 1` and an available terminal. `--output json|jsonl` and `--non-interactive` intentionally disable terminal prompts.
- `commit` uses `-m` and does not open the normal editor, but still follows repository hooks and signing configuration. Interactive hooks or signing programs require text mode and `--jobs 1`; custom programs that access the TTY directly are not fully constrained by `GIT_TERMINAL_PROMPT=0`.
- `--timeout` terminates the directly launched Git child. batch-git reports a timeout but cannot guarantee termination of all authentication, transport, filter, hook, or signing descendants, which may continue running. It also does not limit workspace-lock waits. After a commit timeout, local refs may already have changed; inspect the actual repository state.
- Registered native schedules always run non-interactive Git children. Configure credentials in advance; terminal authentication prompts are unavailable.

Compatibility changes are documented first in the `Unreleased` section of both changelogs. Systems
or Git versions not listed here may work but are not part of the release support commitment.
