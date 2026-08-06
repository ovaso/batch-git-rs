# Agent Collaboration Guidelines

[简体中文](AGENTS.zh-CN.md)

This repository contains a safety-first Rust CLI. Read `README.md` and
`docs/DEVELOPMENT.md` first. Then, as required by the task, read
`docs/ARCHITECTURE.md`, `docs/AUTOMATION_CONTRACTS.md`, and the repository-local
`skills/batch-git-automation/SKILL.md`.

## Working rules

- Keep `workspace.toml` as the only persistent source of truth for a workspace. Do not introduce a second state file.
- Do not make `batch-git` implicitly merge, rebase, stash, reset, clean, force-push, or delete repositories.
- When changing CLI arguments, exit codes, JSON fields, the manifest schema, or schedule behavior, update the help text, both language versions of the relevant user documentation and automation contracts, and both changelogs together.
- When changing v1 receipts, JSONL events, `reason_code`, `capabilities`, `schema`, or plan/apply semantics, update `skills/batch-git-automation/` and add black-box coverage to `tests/automation_protocol.rs`.
- When changing platform scheduler code, preserve the explicit behavioral differences between launchd, systemd, and Windows. Do not put host-specific paths or local usernames in test assertions.
- Prefer behavioral tests over implementation-detail tests. Network, filesystem, and scheduler calls must have replaceable test boundaries.
- Unless the user explicitly requests it, do not run Git commands with remote side effects or register a schedule on the host.

## Validation

At minimum, run the following for Rust changes:

```sh
cargo fmt -- --check
cargo test --locked
cargo clippy --all-targets --all-features -- -D warnings
cargo build --locked --release
```

For documentation or automation changes, also check links and YAML/TOML syntax, and report any platforms that could not be covered locally.
