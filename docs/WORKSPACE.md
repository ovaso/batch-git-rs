# batchspace.toml and Runtime Configuration

[简体中文](zh-CN/WORKSPACE.md)

## 1. Manifest discovery

`batchspace.toml` is the declarative source of truth for a workspace. Normal commands search upward
from the current directory for the nearest manifest. `BATCH_GIT_WORKSPACE` can name an absolute
workspace path. `scan` and `restore` always use the current directory as their root to avoid
modifying a parent workspace accidentally.

### Migrating from the legacy filename

`workspace.toml` is no longer discovered or read. Rename an existing manifest explicitly before
running batch-git; do not keep both filenames in the same workspace:

```sh
mv workspace.toml batchspace.toml
```

## 2. Complete example

```toml
version = 1
created_at = "2026-07-28T12:00:00Z"
updated_at = "2026-07-29T09:30:00Z"

[[repositories]]
name = "service-api"
directory = "services/service-api"
default_branch = "main"
primary_remote = "origin"
created_at = "2026-07-28T12:10:00Z"
synced_at = "2026-07-29T08:30:00Z"

[[repositories.remotes]]
name = "origin"
fetch_url = "git@example.com:team/service-api.git"
push_url = "git@example.com:team/service-api.git"

[[schedules]]
name = "nightly-sync"
enabled = true
action = "sync"
at = "02:30"
overlap = "skip"

[schedules.scope]
all = true
```

## 3. Repository fields

| Field | Required | Description |
|---|---|---|
| `name` | Yes | Repository name, unique within the workspace. |
| `directory` | Yes | Unique workspace-relative directory. Absolute paths, `.` and `..` are forbidden, and the real path must not resolve outside the workspace through a symlink. |
| `default_branch` | Yes | Target of `checkout --default` and source branch for `merge --default`. |
| `primary_remote` | Yes | Primary remote name; must exist in `remotes`. Discovery defaults to `origin`. |
| `created_at` | Yes | RFC 3339 timestamp. |
| `synced_at` | No | Most recent successful synchronization time. |
| `remotes` | Yes | At least one named remote. |
| `fetch_url` | Yes | Fetch URL. |
| `push_url` | No | Separate push URL. Omitting it, or setting it equal to `fetch_url`, clears a separate push URL in the repository. |

The manifest does not store the current branch, every local/remote branch, HEAD, tags, or working-tree
changes. Those dynamic facts always come from each repository's `.git` data.

## 4. Writes and safety boundaries

- The current schema version is fixed at `1`.
- Names and directories must be unique.
- Timestamps must be valid RFC 3339.
- User information is removed from HTTP(S) URLs before writing.
- Writes use a same-directory temporary file, synchronization, and atomic replacement.
- Processes that run Git or modify the manifest use the hidden canonical runtime `.batchspace.lock`; it is retained after exit so all processes coordinate on one stable file, while the operating-system lock is released with the file handle. During v1, they also acquire `.workspace.lock` in a fixed order to remain mutually exclusive with pre-rename clients.
- The program normalizes TOML formatting and does not promise to preserve hand-written comments or original layout.

The entire `schedules` collection may be omitted, which is equivalent to an empty list. Hand-written
schedules may also omit `enabled`, `action`, `timezone`, and `overlap` because they have defaults.
The schema returned by `schema workspace` describes this input form rather than requiring the
defaulted fields emitted by the serializer.

For `batch-git --output json --plan …`, the receipt's `workspace.revision` is a `sha256:<hex>` digest
of the exact `batchspace.toml` bytes. Use it only as the precondition for the immediately following
`--apply --expect-workspace-revision`; do not write it into the manifest or treat it as a schema
field. Apply recomputes the digest after acquiring the lock. Any comment or whitespace change also
invalidates the digest.

Comments therefore should not carry business meaning. After manual edits, validate with a read-only
command:

```sh
batch-git info
```

## 5. Environment variables

Precedence is CLI argument > environment variable > built-in default. Invalid values fail instead of
silently falling back.

Use the following read-only commands to inspect supported variables and their effective values for
the current invocation. They do not require the current directory to belong to a workspace:

```sh
batch-git env ls
batch-git env ls -d
batch-git --output json env list
```

Text output contains `VARIABLE`, `DEFAULT`, and `CURRENT` by default. `-d` / `--description` adds a
`DESCRIPTION` column. `CURRENT` is green in an interactive terminal. Dynamic directories are
resolved to real paths. Without an explicit `BATCH_GIT_WORKSPACE`, the current value shows the
auto-discovered workspace or `<not found>`. Global `--jobs` overrides `BATCH_GIT_JOBS` and is
reflected in the effective value.

| Environment variable | Default | Description |
|---|---:|---|
| `BATCH_GIT_JOBS` | `4` | Maximum concurrent repositories; must be greater than `0`. |
| `BATCH_GIT_SCAN_DEPTH` | `1` | Default scan depth; must be greater than `0`. |
| `BATCH_GIT_WORKSPACE` | Unset | Absolute workspace path used by normal commands. |
| `BATCH_GIT_STATE_DIR` | Platform user state directory | Root for schedule registration state and logs. |
| `BATCH_GIT_SCHEDULE_LOG` | `false` | Whether registered jobs record stdout/stderr. |
| `BATCH_GIT_TZ` | System timezone | Schedule timezone; re-register after changing it. Whitespace is forbidden. |
| `BATCH_GIT_REMOTE` | Unset | Remote used to disambiguate ordinary checkout/merge. `--default` always uses each repository's `primary_remote`. |
| `CURRENT_FEATURE_BRANCH` | Unset | Target for `checkout --feature` / `cf` and source for `merge --feature`. |
| `BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE` | `true` | Whether `merge --default` fetches and merges the newest remote-tracking default branch. |
| `BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT` | `false` | Whether `merge --feature` first fast-forwards the current branch. |
| `BATCH_GIT_PASSTHROUGH_VERBOSE` | `true` | Whether whole-workspace Git passthrough shows successful output. |
| `NO_COLOR` | Unset | Disable color when the variable exists. |

Except for `NO_COLOR`, which is interpreted by presence, Boolean variables accept `1/0`,
`true/false`, `yes/no`, and `on/off`, case-insensitively.

## 6. Local state files

`.batchspace.lock` lives in the workspace root and is the canonical runtime coordination file for potentially conflicting operations. It is not manifest state: batch-git does not delete it after each operation because removing and recreating a lock file introduces a race between concurrent processes. v1 also retains and locks `.workspace.lock` after `.batchspace.lock` only for compatibility with pre-rename clients; both are runtime coordination files, not workspace state.
Schedule registration state and optional logs live in the user state directory, never in the shared
manifest:

- Explicit override: `BATCH_GIT_STATE_DIR`.
- macOS default: the Application Support state directory under the user's Library.
- Windows default: `%LOCALAPPDATA%\batch-git`, falling back to `AppData\Local\batch-git` under the user directory.
- Linux/Unix default: `$XDG_STATE_HOME/batch-git`, or `$HOME/.local/state/batch-git` when unset.

Use `batch-git schedule status <name>` as the source of truth for native task and log paths; do not
guess platform paths in scripts.
