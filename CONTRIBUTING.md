# Contributing

[简体中文](CONTRIBUTING.zh-CN.md)

Thank you for contributing. Before opening an issue or pull request, read the [Code of Conduct](CODE_OF_CONDUCT.md) and the [development guide](docs/DEVELOPMENT.md).

## Submitting changes

The `main` branch is protected. Every repository change, including documentation and workflow-only
changes, must be submitted from a topic branch through a pull request. Direct pushes and release
commits to `main` are not part of the normal workflow.

1. Update local `main` with a fast-forward-only pull, then create a focused topic branch.
2. Discuss substantial changes to the CLI, manifest schema, scheduler semantics, or safety boundaries in an issue or Discussion first.
3. Keep each pull request focused on one purpose. Describe user-visible behavior, platform impact, and the rollback approach.
4. Add a regression test for each bug. For new behavior, cover success, failure, and safety boundaries.
5. Run every quality command in the development guide. Do not commit `target/`, credentials, real private remote URLs, or host-local state files.
6. Update both language versions of the affected README sections, topic guides, automation contracts, and the `Unreleased` section of both changelogs.
7. Push the topic branch, open a pull request, and merge only after the required GitHub Actions checks and review rules pass.

Release tags are not substitutes for pull requests. Prepare version and changelog changes in a PR,
merge them first, verify `main` is green, then create and push the `v*` tag at that exact merged
commit. A manual Release workflow run is only a retry mechanism for an existing remote tag.

## Compatibility commitment

Within the same major version, documented CLI behavior, `batchspace.toml` version 1, legacy `--json` payloads, automation protocol v1, and public schemas only receive backward-compatible extensions. Breaking changes must include a migration plan in the issue or pull request and ship in the next major version.

## Review priorities

Reviews pay particular attention to partial-success reporting across repositories, locking and concurrency, path traversal, credential sanitization, Git interaction behavior, scheduler differences across Windows/macOS/Linux, and consistency between documentation and help text.
