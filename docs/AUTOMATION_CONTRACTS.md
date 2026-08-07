# Automation Contracts

[简体中文](zh-CN/AUTOMATION_CONTRACTS.md)

This document defines the stable interface that `batch-git` exposes to CI, scripts, and agents.
Human-facing tables, colors, and explanations may evolve. Automation must use versioned output and
decide from exit codes and stable fields. Unless a new major version says otherwise, releases within
the same major version may add fields but do not remove fields or change existing field types.

## Selecting an output protocol

New integrations must use the global output option:

```sh
batch-git --output json --request-id task-42 status
batch-git --output jsonl sync --match 'service-*'
```

| Format | Intended use | stdout guarantee |
|---|---|---|
| `text` (default) | Human terminal | Tables, prompts, and required child-process diagnostics. |
| `json` | Request/response agents, CI, and scripts | Exactly one compact JSON document; stderr is empty on success. |
| `jsonl` | Long batch jobs and progress consumers | One independent JSON event per line; no tables, progress bars, or raw Git output. |

`--output json` / `jsonl` implicitly launches Git children non-interactively so TTY prompts cannot
corrupt the protocol. Use `--non-interactive` when text mode must explicitly forbid interaction.
`--timeout 30s`, `5m`, or `1h` limits each system Git child launched by batch-git. It does not limit
workspace-lock waits, local git2 operations, or native scheduler commands. Timeout terminates only the
direct Git child and cannot guarantee termination of authentication, transport, or helper descendants,
which may remain alive after a timeout result.

Add filters and commit hooks or signing programs are also descendants that may survive. A commit may
update a local ref before a later step times out. Automation receiving `timeout` must recheck HEAD and
the index and must not assume no commit was created or retry blindly.

Captured Git stdout and stderr retain at most 1 MiB each. Above the limit, batch-git continues draining
the pipe but keeps only the beginning and end, inserting `[batch-git: output truncated]` between them.
Machine receipts contain no raw Git output. Consumers of text detail or debugging per-repository
results must not assume captured output is complete.

Registered native schedules enter through the hidden `schedule native-run` action, which always
passes `--non-interactive` to the actual `schedule run` child. Authentication that depends on terminal
prompts is unsupported.

Legacy subcommand `--json` preserves its old top-level shape: for example, `list --json` is an object
and `schedule list --json` is an array. It exists only for compatibility; new callers should use
global `--output json`. When both appear, global `--output` determines rendering.

## v1 JSON receipt

`--output json` has this fixed top-level structure:

```json
{
  "api_version": "v1",
  "command": "sync",
  "request_id": "task-42",
  "workspace": {
    "path": "/absolute/workspace",
    "revision": "sha256:..."
  },
  "exit_code": 0,
  "ok": true,
  "data": {
    "summary": { "ok": 2, "skipped": 0, "failed": 0 },
    "results": [{
      "repository": "service-api",
      "directory": "services/service-api",
      "status": "ok",
      "detail": "remote refs updated",
      "synchronized": true
    }]
  },
  "error": null
}
```

Core fields:

| Field | Meaning |
|---|---|
| `api_version` | Currently fixed at `v1`. |
| `command` | Command actually executed; schedule values look like `schedule list` or `schedule run`. |
| `request_id` | Caller-provided correlation ID; omitted when absent. It must contain 1–128 non-control characters. |
| `workspace` | Resolved workspace path and current `workspace.toml` SHA-256 revision; omitted when not applicable. |
| `exit_code` / `ok` | Match the process exit code. For partial repository failure, `ok=false` and `exit_code=1`, but `error=null`. |
| `data` | Command result. Commands may add fields; do not assume one shared data model. |
| `error` | Used only for top-level exit-code-`2` failures; `null` on success or partial repository failure. |

Batch `data.results[]` are emitted in stable manifest order. Each item contains `repository`,
`directory`, `status` (`ok`, `skipped`, or `failed`), `detail`, optional `reason_code`, optional Git
`exit_code`, and `synchronized`. `detail` is human-facing and not a stable decision input; prefer
`status` and `reason_code`.

Common repository reason codes include `dirty_worktree`, `no_upstream`, `branch_ambiguous`,
`branch_missing`, `repository_unavailable`, `nothing_to_push`, `nothing_to_stage`,
`nothing_to_unstage`, `nothing_to_commit`, `unresolved_conflicts`, `detached_head`,
`repository_operation_in_progress`, `timeout`, `git_exit`, and `operation_failed`. Individual
commands may add codes.

Staging and commit commands use these stable classifications:

| code | status | Meaning / action |
|---|---|---|
| `nothing_to_stage` | `skipped` | `add` found no non-ignored working-tree changes to stage. |
| `nothing_to_unstage` | `skipped` | The `unstage` index already matches HEAD; an unborn repository has an empty index. |
| `nothing_to_commit` | `skipped` | `commit` found no staged content; unstaged and untracked content is not added implicitly. |
| `unresolved_conflicts` | `failed` | `add` / `commit` rejected unresolved conflicts; handle the exact repository explicitly. |
| `detached_head` | `failed` | `commit` rejected detached HEAD; check out a local branch first. |
| `repository_operation_in_progress` | `failed` | A merge, rebase, cherry-pick, revert, or similar operation is active; use an explicit Git continue/abort flow. |

## Top-level errors

Argument, environment, manifest, selector, lock, or persistence failures return exit code `2` and a
stable code in `error`:

| code | Meaning / action |
|---|---|
| `invalid_arguments` | Correct the CLI arguments; for example, `commit -m` cannot have an empty message. |
| `workspace_not_found` | Enter a workspace, create a manifest, or set absolute `BATCH_GIT_WORKSPACE`. |
| `workspace_manifest_invalid` | Repair `workspace.toml`. |
| `unknown_repository` / `ambiguous_repository` / `selector_no_match` | Use `list --output json` to obtain canonical names. |
| `stale_workspace_revision` | Plan again and use the new revision. |
| `workspace_locked` | Wait for the current operation and retry. |
| `timeout` | Check connectivity or retry with a larger `--timeout`. |
| `schedule_invalid` | Check the schedule declaration and platform limits. |
| `operation_failed` | Top-level failure with no more specific stable classification. |

These codes come from typed error sources, never from English keyword searches in `message`. Errors
also contain evolvable `message`, `retryable`, and optional `hint` fields. Do not make automation
decisions by matching message text. The machine protocol does not include raw Git stdout/stderr by
default, and sanitizes HTTP(S) URL user-info in machine diagnostics.

## JSON Lines events

`--output jsonl` uses the same `api_version`, `command`, `request_id`, and available `workspace`
fields. Short commands emit at least `started` and a final `finished` with `exit_code` / `ok`. Batch
repository commands emit manifest-ordered `repository_finished` events between them:

```json
{"api_version":"v1","event":"started","command":"sync","data":{"repositories":2}}
{"api_version":"v1","event":"repository_finished","command":"sync","data":{"repository_index":0,"result":{"repository":"service-api","status":"ok"}}}
{"api_version":"v1","event":"finished","command":"sync","exit_code":0,"ok":true,"data":{"ok":2,"skipped":0,"failed":0}}
```

Parsing or initialization failures emit an `error` event with `exit_code=2` and `ok=false`. Consumers
must use the final `finished` (or `error`) plus the process exit code, not intermediate progress, as
the result. `started` appears before batch repository work begins. To preserve the v1 manifest-order
guarantee, a later-index repository that finishes early may wait for every preceding repository event;
event order is not completion-time order.

A single clone uses the complete lifecycle: `started` before system Git, repository index `0` in
`repository_finished`, and final `finished`. If clone fails before registration, the requested
destination is the stable `repository` identifier. `add`, `commit`, and `unstage` use the same
started/repository_finished/finished lifecycle; repositories with nothing to do emit `skipped`.

## Plan and apply

Built-in Git and manifest write operations can be previewed without side effects:

```sh
plan="$(batch-git --output json --plan sync --match 'service-*')"
# Read the revision from plan.workspace.revision.
batch-git --output json --apply \
  --expect-workspace-revision 'sha256:…' \
  sync --match 'service-*'
```

Plan `data` contains `mode=plan`, resolved repository scope, risk, expected side effects,
concurrency, and `workspace_revision`. It does not write the manifest, modify repositories, access
remotes, or register schedulers.

Plans for `add`, `commit`, and `unstage` also contain a fixed first-version `parameters` shape:

| command | `risk` | `side_effects` | `parameters` |
|---|---|---|---|
| `add` | `index` | `["git_indexes"]` | `{"scope":"all_working_tree_changes","includes":["additions","modifications","deletions"],"force_ignored":false}` |
| `commit` | `local_history` | `["git_objects","local_refs","git_indexes","hooks"]` | `{"message":"…","stages_content":false}` |
| `unstage` | `index` | `["git_indexes"]` | `{"scope":"all_staged_changes","preserves_working_tree":true,"moves_head":false}` |

These commands reuse standard repository selectors and target the whole workspace when no selector
is given. The first version accepts no file pathspec. A commit plan includes the caller's message
unchanged; log systems should protect it like normal commit metadata, not treat the plan as secret
storage. Plan does not run add, hooks, signing, or `git read-tree`.

Each `selection.repositories[]` in a merge plan also contains `source_branch`, `source_mode`,
`remote_fallback`, and `source_refresh_remote`. `source_mode` is `explicit`, `feature_environment`, or
`workspace_default`. The last reads each repository's declared `default_branch`, so sources may
differ. `remote_fallback` only describes remote resolution when no local same-named branch exists; it
does not mean plan read or locked the remote ref. `source_refresh_remote` appears only when effective
source refresh is enabled—explicit `--refresh-source` / `--rs`, or the default of `merge --default`—
and identifies the remote apply will fetch and merge.

Ordinary merge `parameters` contain Boolean `update_current` and `refresh_source`. The first maps to
the fast-forward target update from `--update-current` / `--uc`; the second maps to source refresh
from `--refresh-source` / `--rs`. `merge --default` uses each repository's `primary_remote`; an
ordinary merge without an explicit remote also refreshes from `primary_remote`. For
`workspace_default`, the apply workspace-revision precondition prevents the manifest branch or
primary remote from drifting silently after plan. Callers must still preserve explicit arguments and
environment variables between plan and apply.

`BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE` supplies the source-refresh default only for
`merge --default` (default true). `BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT` supplies the current-branch
update default only for `merge --feature` (default false). CLI enable/`--no-*` options win, and the two
plan Booleans always describe effective apply behavior. If `update_current` is true but the current
branch has no upstream, that repository skips the fast-forward pull and continues merging the source;
the parameter records requested behavior, not a guarantee that every repository starts pull.

Apply rechecks the exact `workspace.toml` byte digest after acquiring the workspace lock and before
running Git. It prevents manifest drift between plan and execution but does **not** preserve remote,
HEAD, index, or working-tree state; normal Git runtime checks still apply. Batch operations complete
per repository and provide no cross-repository transactional rollback. A partially successful commit
is not reset, amended, or rebased because another repository's hook, signing, conflict, or Git command
fails.

`--apply` requires `--expect-workspace-revision` and applies only to side-effecting operations. When
creating a workspace for the first time, there is no manifest revision to verify; after explicit
confirmation, run the operation directly. Schedule uses its own `schedule plan`, `doctor`, `generate`,
and command-specific `--dry-run`; global `--plan schedule …` is explicitly rejected to avoid mixing
the two models.

`restore` intentionally uses the invocation's current directory as the workspace root instead of
searching for a parent manifest. Before planning or applying restore, `cd` to the directory containing
`workspace.toml`. If a clone or restore Git child fails or times out, the destination remains for
inspection and is not registered automatically. batch-git does not recursively delete it because a
concurrent process may have written content there. Handle the directory before retrying.

## Capability and schema discovery

Agents must not assume the installed binary equals the repository source. Query it first:

```sh
batch-git --output json capabilities
batch-git schema operation-result
batch-git --output json schema workspace
```

`capabilities` declares binary version, protocol version, output formats, available commands,
plan/apply boundaries, and safety properties. `commands` is the current build's real surface: default
builds include `schedule`; a `--no-default-features` binary neither lists nor accepts it. Callers must
not infer features from the version alone.

Binaries supporting this command group list `add`, `commit`, `env`, and `unstage` in `commands` and
declare `commit_stages_content=false`, `add_rejects_unresolved_conflicts=true`,
`commit_rejects_repository_operations=true`, and `unstage_preserves_working_trees=true` in `safety`.

`schema operation-result` returns the v1 envelope JSON Schema. `schema workspace` returns the JSON
representation schema for `workspace.toml` input, not a serializer-specific format with defaults
filled in. It accepts omitted defaults such as empty `repositories` / `schedules` and schedule
`enabled`, `action`, `timezone`, and `overlap`. The schema uses `additionalProperties: true`, so
consumers should validate core fields while allowing future additions.

The canonical schema identifiers are
`https://github.com/ovaso/batch-git-rs/schemas/operation-result-v1.json` and
`https://github.com/ovaso/batch-git-rs/schemas/workspace-v1.json`. Treat `$id` as the stable schema
identifier returned by the binary; obtain the actual current document through the `schema` command.

## Command coverage and legacy JSON

Every public command—including `env list` / `env ls`, `scan`, `clone`, `restore`, `add`, `commit`,
`unstage`, `fetch`, `sync`, `pull`, `push`, `checkout`, `merge`, `exec`, passthrough, `forget`, and
every schedule subcommand—supports global JSON receipts and JSONL. `status` and `branch` also offer
legacy direct-payload `--json`. For `exec` and top-level `-- <git args>`, batch-git provides only the
structured envelope and marks risk `unclassified`; it does not attempt to classify arbitrary Git
arguments as safe or side-effect-free.

`env list` requires no workspace. Its receipt `command` is fixed at `env list`, and `data.variables`
lists variables in help order. Each item has string fields `name`, `default`, and `current`; string
`description` appears only with `-d` / `--description`. `current` is the effective value for this
invocation, including global `--jobs`, auto-discovered workspace, expanded platform state directory,
and possibly host absolute paths. Invalid environment values still return exit code `2` rather than a
plausible fallback. Automation must read these fields instead of parsing text tables or color.

Legacy JSON shapes:

| Command | Legacy `--json` top-level shape |
|---|---|
| `list`, `find`, `info`, `status`, `branch` | object |
| `schedule list`, `schedule doctor` | array |
| `schedule plan`, `schedule status` | object |

## Agent safety workflow

1. Use `capabilities`, then `info` or `list --output json` to confirm the workspace and selectors.
2. Before and after writes, inventory with `status --output json`, `branch --output json`, or `find --output json`. Before commit, review `git diff --cached` in each exact repository.
3. Use `--plan` before side-effecting Git/manifest operations. Use the command-specific `--dry-run` for push and schedule.
4. Report success, skipped, and failed separately using per-repository `status`, `reason_code`, and the exit code.
5. Commit only the index. Do not combine add and commit into an unreviewed step. Without explicit authorization, do not actually push, merge, register/unregister a scheduler, or bypass safety through passthrough. For `merge --default`, review each repository's planned `source_branch`.
