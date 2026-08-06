# batch-git User Guide

[简体中文](zh-CN/USER_GUIDE.md)

This guide is for everyday users. See [WORKSPACE.md](WORKSPACE.md) for manifest fields,
[SCHEDULES.md](SCHEDULES.md) for complete scheduled-job instructions, and
[AUTOMATION_CONTRACTS.md](AUTOMATION_CONTRACTS.md) for CI and agent field contracts.

## 1. Core concepts

A batch-git workspace looks like this:

```text
workspace/
├── workspace.toml
├── service-api/
├── service-web/
└── service-worker/
```

Every subdirectory is an independent Git repository. `workspace.toml` stores only stable information
required to restore repositories; it does not cache the current branch, HEAD, branch lists, or dirty
state.

Except for `scan` and `restore`, normal commands search upward from the current directory for the
nearest `workspace.toml`. Set the absolute-path variable `BATCH_GIT_WORKSPACE` to select a workspace
explicitly. `scan` and `restore` always treat the current directory as the workspace root.

## 2. Installation and verification

Prerequisites:

- System Git.
- For scheduled jobs: macOS uses `launchd`, Linux requires `systemd --user`, and Windows uses Task Scheduler. The first Windows implementation does not support cron declarations.
- Set schedule timezones with `BATCH_GIT_TZ`; otherwise the system timezone is used.

### 2.1 Prebuilt releases

Official releases support Linux x86_64, macOS x86_64/arm64, and Windows x86_64. Installers require
an explicit `vX.Y.Z` version, verify SHA-256, never invoke `sudo`, and install into a user directory by
default:

```sh
VERSION=vX.Y.Z
curl -LO "https://github.com/livenv/batch-git/releases/download/$VERSION/install.sh"
sh install.sh --version "$VERSION"
```

```powershell
$Version = "vX.Y.Z"
Invoke-WebRequest "https://github.com/livenv/batch-git/releases/download/$Version/install.ps1" -OutFile install.ps1
.\install.ps1 -Version $Version
```

For manual installation, download the matching `.sha256` or combined `SHA256SUMS`. GitHub CLI can
verify build provenance signed by the release workflow:

```sh
gh attestation verify batch-git-<target>.tar.gz --repo livenv/batch-git
```

Archives contain `completions/`. The Unix installer installs Bash, Zsh, and Fish completions. If a
custom prefix is not in the shell's default search path, add `<prefix>/share/zsh/site-functions` to
`fpath` or source the corresponding file directly. PowerShell users can dot-source
`<prefix>\share\batch-git\completions\batch-git.ps1`.

### 2.2 Building from source

Source builds additionally require Rust 1.85 or newer:

```sh
cargo install --locked batch-git
# When cargo-binstall is installed, it can select the prebuilt archive from release metadata:
cargo binstall batch-git

cargo build --release
./target/release/batch-git --help
```

The build installer requires an explicit destination:

```sh
BATCH_GIT_INSTALL_PATH="$HOME/.local/bin" ./build.sh
batch-git --version
```

This documentation always uses the full command name, `batch-git`. Configure `bit` yourself if you
want a shorter alias:

```sh
alias bit='batch-git'
```

## 3. Creating a workspace

### 3.1 Scanning existing repositories

Run in a directory containing multiple repositories:

```sh
batch-git scan
batch-git scan --depth 2
```

`scan` creates or incrementally extends `workspace.toml`; it never removes existing registrations.
The default scan depth is `1` and can be changed with `--depth` or `BATCH_GIT_SCAN_DEPTH`.

### 3.2 Cloning and registering

```sh
batch-git clone git@example.com:team/service-api.git
batch-git clone -b develop --depth 10 --single-branch \
  git@example.com:team/service-web.git services/service-web
```

The manifest is written only after a successful clone. The destination must be a workspace-relative
path and must not already exist. `batch-git clone` is a controlled proxy for system `git clone`: the
options above map to native Git, and authentication, credential helpers, SSH configuration, and
proxy settings use the user's existing Git configuration.

If clone fails or times out, the destination remains for inspection. The tool never recursively
deletes content that another process may have written. Remove or rename that directory explicitly
before retrying the same destination; a failed clone is not registered automatically.

### 3.3 Restoring from a manifest

```sh
cd /path/to/restored-workspace
batch-git restore
batch-git fetch
```

`restore` clones missing repositories only and never overwrites existing repositories. It initially
clones the default branch; run `fetch` afterward to obtain complete remote references. A failed or
timed-out restoration clone also preserves its destination for manual inspection and handling before
retrying.

## 4. Inspecting the workspace

```sh
batch-git list
batch-git list --json
batch-git branch
batch-git branch --json
batch-git status
batch-git status --json
batch-git info
batch-git info service-api
batch-git info services/service-api --json
batch-git env ls
```

- `list`: registered names, current branches, and default branches.
- `branch`: each repository's live current branch without network access.
- `status`: working-tree state, change counts, and upstream differences calculated from local references.
- `info`: workspace metadata or manifest plus live Git facts for one repository.
- `env ls`: supported environment variables, defaults, and effective values for this invocation; no workspace is required.

`env ls` shows `VARIABLE`, `DEFAULT`, and `CURRENT` by default. Add `-d` / `--description` for
`DESCRIPTION`. `CURRENT` is green in a color-capable interactive terminal and still follows
`NO_COLOR`; redirected or piped output stays plain text. Because `--jobs` is global,
`batch-git --jobs 8 env ls` reports the effective `BATCH_GIT_JOBS` value as `8`.

`status` does not list file names. To inspect individual files, use:

```sh
batch-git -- status --short
```

Legacy `info`, `list --json`, `find --json`, `status --json`, `branch --json`, and schedule JSON
output remain suitable for existing scripts. New automation should use the unified global protocol,
for example `batch-git --output json status` or `batch-git --output json env ls`. It emits no table
or progress bar and includes protocol version, exit code, structured errors, and workspace revision
where applicable. See [AUTOMATION_CONTRACTS.md](AUTOMATION_CONTRACTS.md).

Usernames, passwords, and tokens in HTTP(S) remote URLs are removed before storage or display.

## 5. Updating repositories

### 5.1 fetch

```sh
batch-git fetch
```

Fetches and prunes every remote. It does not pull, merge, switch branches, or modify working trees.

### 5.2 sync

```sh
batch-git sync
batch-git sync service-api service-web
batch-git sync --match 'service-*'
batch-git sync --all
```

`sync` is an unattended `restore + fetch`: it first restores selected missing repositories and then
updates remote references. Omitting selectors targets the whole workspace; `--all` expresses the
same scope explicitly.

### 5.3 pull

```sh
batch-git pull
batch-git pull service-api
batch-git pull --match 'service-*'
```

`pull` is always fast-forward-only. A repository must be materialized, HEAD must be a local branch,
the working tree must be clean, the branch must have an upstream, and the update must fast-forward.
The program never stashes, rebases, resets, or resolves divergence automatically.

### 5.4 push

```sh
batch-git push
batch-git push service-api service-web
batch-git push --match 'service-*'
batch-git push --dry-run
batch-git push --set-upstream
batch-git push -u --remote origin
```

`push` sends only each selected repository's current branch, never other branches or tags, and does
not offer force-push. Omitting selectors targets the whole workspace. A branch with an upstream is
pushed to its configured remote branch. Up-to-date branches and branches only behind their upstream
are skipped normally; divergence fails.

A branch without an upstream is skipped by default and no remote branch is created. Only explicit
`-u` / `--set-upstream` pushes to the repository's primary remote and creates tracking; `--remote`
selects another remote. `--dry-run` previews without changing the remote or upstream. Uncommitted
working-tree content is not pushed, but does not prevent committed content from being pushed.

The manifest `push_url` is the source of truth for a separate push address. Omitting it, or making it
equal to `fetch_url`, removes a stale separate push URL from the repository so push falls back to the
fetch URL.

## 6. Branch operations

### 6.1 Searching branches

```sh
batch-git find main
batch-git find 'feature/*'
batch-git find '*login*' --remote
batch-git find 'release/*' --repo 'service-*'
batch-git find '*' --local --json
```

`*` matches zero or more characters. Without `*`, matching is exact and case-sensitive. Remote
results come from existing local remote-tracking references; run `fetch` first when fresh results are
required.

### 6.2 Checking out an existing branch

```sh
batch-git checkout feature/login
batch-git checkout feature/login --remote origin
batch-git cd
batch-git checkout --default
```

Ordinary checkout prefers a local branch. When absent, it creates a tracking branch from the only
same-named remote branch. If multiple remotes contain the same name, use `--remote` or
`BATCH_GIT_REMOTE` to resolve ambiguity. Repositories without the target branch are skipped normally.

`cd` is equivalent to `checkout --default` and checks out the default branch declared by each
repository. It does not fetch, pull, or overwrite the working tree forcibly.

### 6.3 Current feature branch

```sh
export CURRENT_FEATURE_BRANCH='feature/login'
batch-git cf
# Equivalent: batch-git checkout --feature
```

When the variable is unset or empty, the command prints a hint and exits successfully without
reading the workspace.

### 6.4 Creating a branch

```sh
batch-git checkout -b feature/new-api
batch-git checkout -b release/2.0 --from main
batch-git checkout -b hotfix --from release --remote origin
```

By default the branch starts at each repository's current HEAD. `--from` may resolve to a local
branch, the only matching remote branch, a tag, or a commit. A repository fails if the new branch
already exists, the starting point is missing or ambiguous, or the working tree prevents a safe
switch. Force recreation with `-B` is unsupported.

### 6.5 Merging

```sh
batch-git merge feature/login
batch-git merge --feature
batch-git merge --default
batch-git merge --update-current feature/login
batch-git merge --uc --rs --default
batch-git merge --no-update-current feature/login
batch-git merge --no-refresh-source feature/login
```

`merge` merges a source branch into each repository's current branch. Except for the source-refresh
default of `merge --default`, it does not access the network by default. If local tracking references
show that the current branch is behind or diverged, default mode fails before starting Git merge to
avoid leaving an in-progress merge. Run `batch-git fetch` to refresh that judgment and then
`batch-git pull`, or use explicit `--update-current`, whose short alias is `--uc`; it first performs a
fast-forward-only pull of the current tracking branch.

`--refresh-source` / `--rs` fetches declared remotes and resolves the source to the latest
remote-tracking branch. `--default` always uses each repository's `primary_remote`. Ordinary merge
prefers `--remote` / `BATCH_GIT_REMOTE`, otherwise `primary_remote`. The local source branch is not
moved. The switches can be combined: from a `test` or `dev` target,
`batch-git merge --uc --rs feature/login` first fast-forwards the current branch and then merges the
latest remote feature source. If the current target has no upstream, `--uc` safely skips that pull and
continues merging the source. On conflict, batch-git never runs `git merge --abort`; inspect and
resolve the repository manually.

`merge --feature` uses `CURRENT_FEATURE_BRANCH` as the source and requires no positional branch.

`BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE` controls whether `merge --default` fetches and merges the
latest remote default branch; it defaults to `true`.
`BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT` controls whether `merge --feature` first fast-forwards the
current branch; it defaults to `false`. These are mode-specific defaults, not generic argument
mappings. CLI options win: `--uc` / `--update-current` and `--rs` / `--refresh-source` enable the
behaviors, while `--no-update-current` and `--no-refresh-source` disable them. The earlier generic
variables `BATCH_GIT_MERGE_UPDATE_CURRENT` and `BATCH_GIT_MERGE_REFRESH_SOURCE` have been removed.

`merge --default` reads each repository's own `default_branch` from `workspace.toml`, making it
suitable for workspaces with different default names or multi-component branch paths. Resolution
prefers a local branch. If absent, only that repository's `primary_remote` is searched. Without
source refresh it does not fetch, and a separate `batch-git fetch` does not update an existing local
default branch. To consume current remote content, update the local default branch explicitly or use
`--rs` to merge the latest remote-tracking default directly. A repository already on its declared
default branch is skipped normally.

`--default` conflicts with a positional branch, `--feature`, and `--remote`, but can be combined with
`--update-current` / `--no-update-current` and `--refresh-source` / `--no-refresh-source`.

## 7. Staging and committing

`add`, `commit`, and `unstage` use the same repository selectors as `sync`: positional values are
canonical repository names or workspace-relative directories, `--match` uses a case-sensitive `*`
pattern on names, and `--all` explicitly selects the entire workspace. Omitting selectors also
targets the entire workspace. The first version accepts no file pathspec; every selected repository
uses the command's complete index scope.

### 7.1 Staging all working-tree changes

```sh
batch-git add
batch-git add service-api service-web
batch-git add --match 'service-*'
batch-git add --all
```

`add` stages every non-ignored addition, modification, and deletion, equivalent to controlled
`git add --all` from the repository root. It never force-adds ignored files. A repository with
nothing to stage is `skipped`. Unresolved conflicts fail with `unresolved_conflicts`, preventing a
batch command from accidentally marking conflict files as resolved.

To stage specific files during conflict resolution, or whenever file-level staging is required, use
native Git in one precise repository, for example:

```sh
batch-git exec service-api -- add -- src/api.rs
```

### 7.2 Reviewing and committing the index

Keep staging, review, and commit as three explicit steps:

```sh
batch-git add --match 'service-*'
batch-git exec --match 'service-*' -- diff --cached --stat
batch-git commit --match 'service-*' -m 'Update generated clients'
```

`commit` requires a non-empty `-m` / `--message` and uses the same message in every selected
repository with staged content. It commits the current index only, never stages working-tree content
implicitly, and offers no amend, empty-commit, or hook-bypass option. A repository with only unstaged
or untracked changes is skipped with `nothing_to_commit`. An unborn branch can create a root commit
when content is staged.

Safe batch commit rejects detached HEAD, unresolved conflicts, and in-progress merge, rebase,
cherry-pick, revert, or other Git operations. This prevents an ordinary batch command from
accidentally completing an existing Git workflow. Inspect that repository and use an explicit native
Git continue/abort flow.

Commit still follows repository identity, hooks, `core.hooksPath`, and signing configuration.
Pre-commit hooks, commit-msg hooks, and signing programs may fail, wait for interaction, or have
additional side effects. Use text mode and `--jobs 1` when interaction is required. Machine output
and `--non-interactive` close Git stdin and terminal credential prompts, but custom hooks or signing
programs may still access a TTY directly.

### 7.3 Unstaging everything

```sh
batch-git unstage
batch-git unstage service-api service-web
batch-git unstage --match 'service-*'
```

`unstage` restores every index change to HEAD without changing working-tree files or moving HEAD.
New files remain in the working tree and become untracked; staged modifications or deletions become
unstaged. With no commit in an unborn HEAD, the command runs `git read-tree --empty` to clear the
index while preserving the working tree. No staged content is a normal `nothing_to_unstage` skip.

Unstaging removes the current index snapshot and cannot undo an existing commit. If staged content
differs from the working tree, inspect the version to preserve with `git diff --cached` first.

### 7.4 Concurrency and partial success

All three commands hold the workspace lock but use bounded `--jobs` concurrency across different
repositories. They are not cross-repository transactions. A hook, signing program, index lock, or Git
failure in one repository does not roll back staging, unstaging, or commits already completed in
others. In particular, exit code `1` from commit may mean some repositories already contain new
commits; batch-git never resets, amends, or rebases them automatically.

`--timeout` terminates only the direct Git child. Hook, signing, or filter descendants may continue,
and commit may update a ref before a later step times out. After timeout, inspect HEAD, the index, and
the working tree in every repository instead of retrying blindly. batch-git continuously drains Git
stdout/stderr to prevent pipe deadlocks, but retains at most 1 MiB per stream. Large output preserves
the beginning and end with a truncation marker. Redirect explicitly within one precise repository
when complete `log`, `diff`, or diagnostics are required.

## 8. Running native Git

All materialized repositories:

```sh
batch-git -- status --short
batch-git -- log -1 --oneline
```

Selected repositories:

```sh
batch-git exec service-api -- status
batch-git exec service-api service-web -- pull --ff-only
batch-git exec --match 'service-*' -- fetch --prune
```

Positional `exec` repositories are exact names or relative directories. Multiple selectors form a
deduplicated union. `--` is required; subsequent arguments bypass shell parsing and go directly to
system Git.

Whole-workspace passthrough shows successful command output by default; `exec` shows only failures.
Global `--verbose` makes `exec` show successful output. Global passthrough options must appear before
the separator:

```sh
batch-git --jobs 8 --verbose -- status
batch-git --jobs 1 exec service-api -- rebase -i HEAD~3
```

Concurrent children do not receive stdin. Use `--jobs 1` for interactive Git commands. In an
interactive terminal, a single passthrough task gives Git direct access to stdin, stdout, and stderr.
A passthrough `git commit` that clearly reports “nothing to commit” is skipped instead of causing an
aggregate failure.

For unattended use, pass `--non-interactive`; it closes stdin and sets `GIT_TERMINAL_PROMPT=0`.
`--timeout 30s`, `5m`, or `1h` limits each system Git child. `--output json` and `--output jsonl`
automatically use non-interactive child policy so Git prompts cannot corrupt structured stdout.
Timeout terminates only the directly launched Git process, not necessarily authentication,
transport, or helper descendants. Inspect the relevant remote or local directory after timeout; do
not assume every descendant stopped.

## 9. Registration management

```sh
batch-git forget service-old
batch-git forget services/legacy
```

`forget` removes registrations from `workspace.toml` only. It never deletes repository directories or
Git data.

## 10. Concurrency, output, and color

Multi-repository tasks use bounded concurrency but always report results in manifest order. One
repository failure does not cancel the others. Concurrency precedence is
`--jobs` > `BATCH_GIT_JOBS` > `4`.

In interactive terminals, clone, restore, and fetch show dynamic progress. Redirected output and CI
fall back to stable tables. Color control codes are disabled when `NO_COLOR` is set, `TERM=dumb`, or
output is piped.

### 10.1 Programmable output, plan, and apply

```sh
batch-git --output json capabilities
batch-git --output json --request-id build-17 sync --match 'service-*'
batch-git --output jsonl fetch

# Copy receipt.workspace.revision into the next command.
batch-git --output json --plan pull service-api
batch-git --output json --apply --expect-workspace-revision 'sha256:…' pull service-api
```

`--output json` emits exactly one v1 receipt. `--output jsonl` emits one event per line, with
repository events ordered by manifest position rather than completion time. A single clone also emits
one `repository_finished`; on failure, the requested destination identifies the result. `--plan`
performs no writes or network access and lists effective scope, risk, and expected side effects.
`--apply` verifies the `workspace.toml` revision before execution. This cannot lock remote state,
HEAD, the index, or the working tree, so runtime Git checks and per-repository results remain final.
Schedule has separate `schedule plan`, `doctor`, `generate`, and `--dry-run` flows.

Discover contracts from the current binary instead of guessing the installed version:

```sh
batch-git --output json capabilities
batch-git schema operation-result
batch-git --output json schema workspace
```

The workspace schema describes the JSON input representation of `workspace.toml`; `repositories`,
`schedules`, and defaulted schedule fields may be omitted.

## 11. Exit codes

| Exit code | Meaning |
|---:|---|
| `0` | Command completed. Expected skips such as nothing to stage, commit, or unstage are allowed. |
| `1` | At least one repository operation failed. |
| `2` | Argument, configuration, workspace, manifest-validation, or file-write error. |

Batch commands may partially succeed. For exit code `1`, use the repository-level result table as the
source of truth.

## 12. Troubleshooting

### `workspace.toml` cannot be found

Confirm the current directory is inside the workspace or set an absolute path:

```sh
export BATCH_GIT_WORKSPACE='/absolute/path/to/workspace'
batch-git info
```

### A newly created remote branch is missing from search

`find --remote` does not access the network. Run `batch-git fetch` before searching.

### Checkout reports an ambiguous remote branch

```sh
batch-git checkout feature/login --remote origin
```

Alternatively set `BATCH_GIT_REMOTE=origin`.

### Pull fails while fetch succeeds

`pull` has stricter safety conditions. Use `batch-git status` to inspect dirty state, detached HEAD,
upstream, and divergence, then handle the failing repository.

### Push skips a branch without an upstream

Remote branches are not created automatically. After confirming the first push is intended, run:

```sh
batch-git push --set-upstream
# Or select the remote.
batch-git push -u --remote origin
```

### A command appears to wait indefinitely

Processes that run Git or write the manifest serialize on `.workspace.lock`. Check for another
batch-git task or a scheduled job configured with `queue`.

`--timeout` limits only an already-started direct Git child, not the wait for `.workspace.lock`, and
cannot guarantee termination of Git authentication, transport, or helper descendants. Callers that
must bound lock waits should apply their own whole-process timeout or coordinate tasks before
invocation.
