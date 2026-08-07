---
name: batch-git-automation
description: Safely and audibly automate development and maintenance in batch-git multi-repository Git workspaces. Use for capability discovery, batchspace.toml state inspection, JSON receipt or JSONL orchestration of batch staging, commits, and Git operations, plan-before-apply workflows, cross-repository branch/sync/push coordination, or scheduled synchronization when users mention batch-git, multi-repository Git, batchspace.toml, batch synchronization, or agent/CI automation.
---

# Batch Git Automation

[简体中文](SKILL.zh-CN.md)

Use the `batch-git` CLI to manage independent Git repositories declared by `batchspace.toml`. Treat
the manifest as the source of truth for restoration and remote configuration, and each repository's
Git state as the live source of truth. Do not install this skill, start a daemon, or introduce MCP;
this skill only orchestrates the existing CLI.

Read the [capability roster](references/capability-roster.md) first. For field, event, or
compatibility details, read the repository's
[automation contracts](../../docs/AUTOMATION_CONTRACTS.md). Repository source may differ from the
binary installed for the user, so do not infer arguments or protocol from this file alone.

## Default workflow

1. Run `batch-git --output json capabilities` in the target workspace to confirm protocol version, commands, and plan/apply limits. When diagnosing runtime configuration, run `batch-git --output json env list` for effective environment values instead of parsing colored tables. `commands` reflects compiled features; do not attempt schedule workflows when `schedule` is absent.
2. Use `batch-git --output json info` and `batch-git --output json list` to confirm the workspace root and canonical repository names. Set absolute `BATCH_GIT_WORKSPACE` when the path is ambiguous.
3. Before and after any local, remote, or scheduler write, inventory with `status --output json`, `branch --output json`, and when needed `find '<pattern>' --output json`. Before commit, review `git diff --cached` in every exact repository. Record `ok`, `skipped`, and `failed` separately.
4. For write-capable Git/manifest operations, run `--output json --plan` first and verify `data.selection`, risk, expected side effects, and `workspace.revision`. For merge plans, verify each repository's `source_branch`, `source_mode`, `remote_fallback`, and `source_refresh_remote`. Plan does not access remotes and is not a cross-repository transaction.
5. Only after explicit user authorization for the actual change, run the same exact command with `--apply --expect-workspace-revision <plan revision>`. Plan again after every manifest change.
6. Report the new JSON receipt and exit code. Do not parse tables, color, natural-language `detail`, or raw Git output.

For first-workspace `scan` / `clone`, there may be no manifest revision for apply to verify. Show the
plan, request explicit authorization, then run directly; never fabricate a revision.

`restore` intentionally treats the invocation's current directory as the workspace root. Before
planning or applying it, enter the directory containing `batchspace.toml`; do not invoke it from a
child repository.

## Output discipline

New automation defaults to global `--output json`, which emits exactly one v1 receipt on stdout and
uses stable `error.code` for top-level failures. Optional `--request-id <opaque-id>` is echoed into
receipts and JSONL events.

```sh
batch-git --output json --request-id change-184 sync --match 'service-*'
batch-git --output json --plan pull service-api
batch-git --output json --apply --expect-workspace-revision 'sha256:…' pull service-api
```

- For batch or long-running clone, restore, add, commit, unstage, fetch, sync, and pull, use `--output jsonl` when appropriate. Consume `started`, `repository_finished`, and `finished` line by line and decide from the final event plus process exit code. A failed clone uses the requested destination as its stable repository identifier. `started` appears before repository work. Repository events follow manifest order rather than completion order, so a later repository may finish but wait for earlier events.
- Legacy subcommand `--json` exists only for old scripts. Do not treat it as the same schema as new receipts; global `--output` wins when both appear.
- Machine mode disables Git terminal interaction implicitly. For unattended text mode, use `--non-interactive` and, when needed, `--timeout 30s`, `5m`, or `1h`. Timeout terminates only the direct system Git child, not lock waits, and cannot guarantee termination of authentication, transport, filter, hook, or signing descendants. After commit timeout, local refs may already have changed; recheck HEAD and the index instead of retrying blindly.
- Decide from `status`, `reason_code`, `error.code`, and exit code. `detail` and `error.message` are human diagnostics and may evolve. Top-level codes come from typed errors; never reclassify by matching message keywords.
- Captured Git stdout/stderr may retain only head and tail after 1 MiB, with a truncation marker. Machine receipts contain no raw Git output. When complete logs are required, use a user-authorized logging method scoped to the exact repository.
- Registered schedules with `BATCH_GIT_SCHEDULE_LOG=true` write human diagnostic boundaries to both streams: local RFC 3339 start/end timestamps with a numeric UTC offset, duration, action, jobs, and exit code. Treat them as evolving text diagnostics, not a protocol; decide from JSON receipts and process exit codes.

## Selection and safety boundaries

Prefer exact names or workspace-relative directories. `--match` and `find --repo` support only the
case-sensitive `*` wildcard. Copy canonical names from `list --output json`; avoid an unverified broad
`--match '*'`.

Preferred safety gradient: read-only inventory → `fetch` → `sync` → `add` / `unstage` → review staged
diff → `commit` → `pull` → `checkout` / `merge` / `push`. `sync` is suitable for unattended use: it
restores missing repositories and updates remote references without modifying existing working trees.
`pull` is fast-forward-only and never stashes, rebases, resets, or resolves conflicts.

After clone or restore failure/timeout, the destination remains for manual inspection. Do not assume
a retry overwrites or cleans it. Inspect and handle the directory before using the same destination.
The real path of every manifest repository must remain inside the workspace. Never use a symlink to
point a repository or clone/restore destination outside it. For `workspace_manifest_invalid`, inspect
relative paths and symlink boundaries first.

`exec` and top-level `batch-git -- <git-args...>` provide only a structured result envelope and mark
risk as `unclassified`. Use low-risk read-only Git commands by default. Destructive, history-rewriting,
or remote-write commands require explicit user authorization for the exact repositories, arguments,
and impact. Global options must precede the passthrough separator.

## Staging and local commits

- `add`, `commit`, and `unstage` accept canonical repository names, workspace-relative directories, repeatable `--match`, or `--all`. Omitting selectors targets the entire workspace. The first version accepts no file pathspec.
- `add` stages every non-ignored addition, modification, and deletion, never force-adds ignored files, and rejects unresolved conflicts. `nothing_to_stage` is a normal skip. For file-level conflict handling, use explicit native Git in the exact repository.
- After add, review `git diff --cached` per repository before `commit -m <message>`. Commit uses the index only, never implicitly adds, amends, creates empty commits, or bypasses hooks. `nothing_to_commit` is a normal skip.
- Commit rejects detached HEAD, unresolved conflicts, and in-progress merge, rebase, cherry-pick, revert, or other Git operations. Handle `detached_head`, `unresolved_conflicts`, and `repository_operation_in_progress` separately; never use a batch command to continue or abort automatically.
- `unstage` restores the complete index while preserving every working-tree file and not moving HEAD. An unborn HEAD uses `git read-tree --empty`; `nothing_to_unstage` is a normal skip. It cannot undo an existing commit.
- Commit follows each repository's identity, hooks, `core.hooksPath`, and signing configuration. Use text mode with `--jobs 1` for interaction. Machine receipts do not include hook or raw Git output. Partial batch success never resets, amends, rebases, or rolls back commits in other repositories.
- Add/unstage plans use risk `index` and side effect `git_indexes`; commit uses risk `local_history` and side effects `git_objects`, `local_refs`, `git_indexes`, and `hooks`. Apply verifies only the workspace revision, not HEAD, the index, or the working tree.

Use two separate plan/apply flows and review the index between them. Each apply copies the
`workspace.revision` from its immediately preceding plan. Commit apply must repeat exactly the
selector and message used in its plan:

```sh
# 1. Plan and apply complete staging.
batch-git --output json --plan add --match 'service-*'
batch-git --output json --apply --expect-workspace-revision 'sha256:<add-plan-revision>' \
  add --match 'service-*'

# 2. Review the actual index before commit.
batch-git exec --match 'service-*' -- diff --cached --stat

# 3. Plan and apply commit with the same message; copy the commit plan's own revision.
batch-git --output json --plan commit --match 'service-*' -m 'Update generated clients'
batch-git --output json --apply --expect-workspace-revision 'sha256:<commit-plan-revision>' \
  commit --match 'service-*' -m 'Update generated clients'
```

## Remotes, branches, and schedules

- Run `push --dry-run` before actual push. Use `--set-upstream` only with explicit authorization and never force-push.
- Inspect working trees and branches before checkout; pass `--remote` when a remote branch name is ambiguous. Merge requires an explicit source mode (positional branch, `--feature`, or `--default`), scope, and authorization. `--update-current` / `--uc` can only fast-forward the current target. `--refresh-source` / `--rs` fetches and merges the newest remote-tracking source without moving the local source branch. A local-only current target with no upstream skips pull under `--uc` and continues merging. `--default` reads each manifest default branch and, without `--rs`, falls back to `primary_remote` only when no local source exists. `BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE` (default true) affects only `merge --default`; `BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT` (default false) affects only `merge --feature`. CLI enable or `--no-*` options win. Never resolve, abort, or continue conflicts automatically.
- Schedule uses its dedicated flow: `schedule plan` → `doctor` → `generate` or `register --dry-run` → explicitly authorized `register`. Do not use global `--plan schedule …`; it is rejected to avoid mixing models. `register`, `unregister`, and `remove --unregister` modify the native scheduler and require separate authorization.
- A registered schedule's hidden `native-run` child always carries `--non-interactive`. Configure usable non-interactive credentials before registration; do not depend on Git terminal prompts.

## Reporting

Report the workspace, selection, exact command, receipt request ID if present, per-repository status
and reason code, final exit code, and any required manual action. Never copy credentials, private
remote URL user-info, or raw Git child output into reports, commands, or structured data.
