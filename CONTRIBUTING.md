# Contributing

[简体中文](CONTRIBUTING.zh-CN.md)

Thank you for contributing. Before opening an issue or pull request, read the [Code of Conduct](CODE_OF_CONDUCT.md) and the [development guide](docs/DEVELOPMENT.md).

## Submitting changes

1. Discuss substantial changes to the CLI, manifest schema, scheduler semantics, or safety boundaries in an issue or Discussion first.
2. Keep each pull request focused on one purpose. Describe user-visible behavior, platform impact, and the rollback approach.
3. Add a regression test for each bug. For new behavior, cover success, failure, and safety boundaries.
4. Run every quality command in the development guide. Do not commit `target/`, credentials, real private remote URLs, or host-local state files.
5. Update both language versions of the affected README sections, topic guides, automation contracts, and the `Unreleased` section of both changelogs.

## Compatibility commitment

Within the same major version, documented CLI behavior, `workspace.toml` version 1, legacy `--json` payloads, automation protocol v1, and public schemas only receive backward-compatible extensions. Breaking changes must include a migration plan in the issue or pull request and ship in the next major version.

## Review priorities

Reviews pay particular attention to partial-success reporting across repositories, locking and concurrency, path traversal, credential sanitization, Git interaction behavior, scheduler differences across Windows/macOS/Linux, and consistency between documentation and help text.
