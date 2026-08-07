# batch-git Automation Capability Roster

[简体中文](zh-CN/capability-roster.md)

This roster reflects automation protocol v1 in the current source; an installed binary may be older.
Start every automation flow with `batch-git --output json capabilities`, then run
`batch-git schema operation-result` when needed. Do not infer available arguments from the skill,
README, or version alone. Default builds list `schedule`; minimal builds may omit it. The schedule
capabilities below apply only when `commands` actually contains `schedule`.

## Automation foundations

| Capability | Command | Agent rule |
|---|---|---|
| Capability discovery | `capabilities` | Starting point in every new environment; read protocol version, formats, commands, and safety boundaries. |
| Schema discovery | `schema operation-result`, `schema workspace` | Validate core receipt fields while allowing future optional fields. The workspace schema accepts omitted default manifest fields; canonical v1 `$id` values retain the legacy `https://github.com/livenv/batch-git/schemas/` namespace as stable protocol identifiers. |
| Single receipt | `--output json <command>` | Default for new automation; stdout contains exactly one v1 JSON document. |
| Long-task events | `--output jsonl <command>` | Consume `started`, `repository_finished`, `finished`. Batch operations and single clone emit repository terminal events in manifest order, not completion order. Decide from the final event and exit code. |
| Correlation | `--request-id <id>` | Echo the caller's 1–128-character non-control ID into all machine output. |
| Interaction/time control | `--non-interactive`, `--timeout 30s|5m|1h` | Disable prompts for unattended text operations. Timeout terminates only the direct Git child, does not limit workspace-lock waits, and may leave helper descendants alive. |
| Preview and precondition | `--plan` → `--apply --expect-workspace-revision <digest>` | Read plan scope, risk, and revision first. Plan again after manifest changes. Apply does not preserve remote, HEAD, index, or working-tree state. |

Legacy subcommand `--json` remains for old scripts but is not a unified schema. New callers use global
`--output json`; global output wins when both appear.

| Capability | Command | Automation value | Safety level | Agent rule |
|---|---|---|---|---|
| Workspace discovery and validation | `info`, `list` | Determine manifest root, repository names, and machine-readable inputs | Read-only | Start every flow here; prefer `--output json`. |
| Runtime configuration discovery | `env list` (alias `env ls`) | List supported variables, defaults, and effective values | Read-only | No workspace required. Read `data.variables[]`; add `-d` for descriptions. Do not parse green text columns. Output may contain host absolute paths. |
| Runtime inventory | `status`, `branch`, `find` | Identify dirty state, ahead/behind, current branches, and target coverage | Read-only | Run before and after every local or remote write. `find --remote` queries fetched references only. |
| Complete staging | `add [selectors/--match/--all]` | Write all selected non-ignored additions, modifications, and deletions to indexes | Local index write | Omitting selectors means the whole workspace. The first version has no file pathspec and rejects unresolved conflicts. Review staged diffs afterward. |
| Local commit | `commit [selection] -m <message>` | Commit each repository's existing index with one message | Local history write, high risk | No implicit add/amend/empty/hook bypass. Rejects detached HEAD, conflicts, and in-progress Git operations. Hooks/signing can cause partial failure. |
| Complete unstage | `unstage [selectors/--match/--all]` | Restore selected indexes while preserving working trees | Local index write | No file pathspec. Does not move HEAD; unborn uses read-tree empty. Cannot undo commits. |
| Build manifest from existing repositories | `scan [--depth N]` | Convert scattered repositories into a reproducible workspace | Manifest write | Adds registrations only. Plan first and confirm the root. |
| Controlled repository addition | `clone <url> [directory]` | Register one repository only after successful clone | Local write + network | Destination must be a missing relative path inside the workspace. Failure/timeout preserves it for inspection and never recursively deletes it. Handle it before retrying. JSONL emits the full lifecycle and uses the requested directory as the failed repository identity. Never expose URL credentials. |
| Missing repository restoration | `restore` | Recreate missing checkouts from the manifest | Local write + network | Does not overwrite existing directories. Failed/timed-out destinations remain for inspection. Invoke only from the workspace root containing `batchspace.toml`. |
| Metadata maintenance | `forget <selector>` | Stop managing a repository while preserving its directory | Manifest write | Plan the exact match before confirmation. |
| No-working-tree update | `fetch` | Update and prune all remote references | Network, low risk | No merge or checkout; suitable before automation decisions. |
| Unattended synchronization | `sync [selectors/--match/--all]` | `restore + fetch` for missing repositories and refs | Local write + network, low risk | Preferred background operation; does not modify existing working trees. |
| Fast-forward update | `pull [selectors/--match]` | Fast-forward current tracking branches | Working-tree write + network | Use only after confirming scope and clean state; never bypass failures. |
| Safe publication | `push [selection] [--dry-run] [-u/--set-upstream]` | Publish current branches in batches | Remote write | Dry-run first; actual push requires user authorization. No force-push. |
| Target branch switch/create | `checkout`, `cd`, `cf` | Switch same-named, default, or feature branches across repositories | Working-tree write | Inspect status and branches first. Remote ambiguity requires explicit `--remote`. |
| Cross-repository merge | `merge <branch>`, `merge --feature`, `merge --default` | Merge one source or each declared default into current branches | Working-tree write, high risk | Requires explicit source, scope, and authorization. `--uc` fast-forwards targets first; `--rs` fetches and merges newest remote-tracking sources. Leave conflicts for manual handling. |
| Native Git escape hatch | `-- <git-args>`, `exec ... -- <git-args>` | Cover read-only or precise Git actions without a built-in command | Depends on supplied command | Preserve `--`; allow only low-risk read-only commands by default. |
| Schedule declarations (optional feature) | `schedule add/update/list/plan/doctor/generate` | Plan cross-platform synchronization from reviewable declarations | Manifest write or read-only | Use only when capabilities lists `schedule`. Prefer `sync`. Use schedule's own plan, doctor, and generate, never global `--plan schedule …`. |
| Scheduler lifecycle (optional feature) | `schedule register/unregister/remove/run/status` | Register, inspect, run, or clean native scheduler tasks | External system write | Use only when capabilities lists `schedule`. `register`, `unregister`, and `remove --unregister` require explicit authorization; dry-run/generate first. Registered hidden native-run children are non-interactive. |

## Selectors and concurrency

- `add`, `commit`, `unstage`, `sync`, `pull`, and `push` accept exact names/relative directories, repeatable `--match`, or `--all`; omitting selectors means the whole workspace. The three staging commands initially accept no file pathspec.
- `exec` requires at least one exact selector or `--match`; use `batch-git -- <git args>` for whole-workspace passthrough.
- `find --repo` filters canonical repository names only. Wildcards support only case-sensitive `*`.
- Concurrency precedence is `--jobs`, `BATCH_GIT_JOBS`, then the available logical CPU count (falling back to `1` when unavailable). Stable output order does not prevent parallel writes from changing multiple repositories at once.

## Key guarantees and boundaries

- Hidden runtime lock `.batchspace.lock` serializes processes that run Git or modify the manifest. The file remains after exit, while the operating-system lock is released with the process file handle. During v1 it is acquired before the legacy `.workspace.lock`, preserving mutual exclusion with pre-rename clients.
- `pull` is fast-forward-only. batch-git never automatically merges, rebases, stashes, resets, or cleans working trees.
- `add` rejects unresolved conflicts. `commit` commits the index only and rejects detached HEAD and in-progress Git operations. `unstage` changes only the index and preserves the working tree. Capabilities `safety` declares `commit_stages_content=false`, `add_rejects_unresolved_conflicts=true`, `commit_rejects_repository_operations=true`, and `unstage_preserves_working_trees=true`.
- `push` sends only current branches; only `--set-upstream` creates a remote branch and tracking relationship.
- Missing checkout targets are skips; multiple same-named remote branches require disambiguation.
- Batch tasks may partially succeed. Decide from per-repository results and the final exit code, never one summary line.
- Exit codes: `0` completed (expected skips allowed), `1` at least one repository operation failed, `2` argument/configuration/manifest/file error.
- In `--output json`, exit code `1` still includes complete `data.results[]`. For exit code `2`, read top-level `error.code`, not message text. Common repository codes include `dirty_worktree`, `no_upstream`, `branch_ambiguous`, `branch_missing`, `repository_unavailable`, `nothing_to_stage`, `nothing_to_unstage`, `nothing_to_commit`, `unresolved_conflicts`, `detached_head`, `repository_operation_in_progress`, `timeout`, and `git_exit`.
- Every public built-in and schedule subcommand supports v1 JSON/JSONL. Structured results for `exec` and top-level passthrough have `unclassified` risk and must not automatically lower authorization standards.
- Hook, filter, signing, and helper descendants may continue after direct Git-child timeout, and commit refs may already be updated. Partial batch success never automatically resets, amends, rebases, or rolls back across repositories.
