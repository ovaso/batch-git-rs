# Scheduled Synchronization Guide

[简体中文](zh-CN/SCHEDULES.md)

batch-git can register manifest schedules with macOS `launchd`, Linux `systemd --user`, or Windows
Task Scheduler. Prefer `action = "sync"`: it restores missing repositories and updates remote
references without modifying existing working trees.

## 1. Standard workflow

```sh
batch-git schedule add nightly-sync --at 02:30 --all
batch-git schedule plan nightly-sync
batch-git schedule doctor nightly-sync
batch-git schedule register nightly-sync
batch-git schedule status nightly-sync
```

After changing a declaration, register it again to update the native task:

```sh
batch-git schedule update nightly-sync --at 03:00
batch-git schedule register nightly-sync
```

`register` is an idempotent upsert: it returns `unchanged` when the configuration is identical and
updates the same task when it changes.

For CI or agents, use the global protocol with every schedule subcommand instead of parsing prompts:

```sh
batch-git --output json schedule plan nightly-sync
batch-git --output json schedule register nightly-sync --dry-run
batch-git --output jsonl schedule run nightly-sync
```

New receipts use `schedule <action>` as `command` and place the legacy arrays from
`schedule list --json` and `schedule doctor --json` directly in `data`. Legacy subcommand `--json`
keeps its original shape. `--output json` suppresses native `launchctl`, `systemctl`, and
`schtasks.exe` output so it cannot corrupt the protocol.

## 2. Creating a declaration

Run every day in local time:

```sh
batch-git schedule add nightly-sync --at 02:30 --all
```

Run at a fixed interval:

```sh
batch-git schedule add backend-sync \
  --every 30m \
  --repo service-api \
  --repo service-worker
```

Use a six-field cron expression:

```sh
batch-git schedule add business-hours \
  --action pull \
  --cron '0 */15 9-17 ? * MON-FRI' \
  --repo service-api
```

`add` requires exactly one of `--at`, `--every`, or `--cron`, and one scope form: `--all` or one or
more repeatable `--repo` values. Workspace-relative repository directories are normalized to
repository names when written.

Important options:

- `--action sync|pull`: defaults to `sync`.
- `--overlap skip|queue`: skip or wait when another workspace operation holds the lock; defaults to `skip`.
- `--disabled`: create a disabled declaration. Disabled schedules cannot be planned, run, or registered.

Set the schedule timezone with the project-specific `BATCH_GIT_TZ` variable:

```sh
BATCH_GIT_TZ=Asia/Shanghai batch-git schedule register nightly-sync
```

When unset or empty, no timezone override is injected and the operating-system timezone is used.
After changing the variable, run `schedule register` again so the native definition and registration
summary are updated. Use a whitespace-free identifier supported by the operating system, such as the
IANA timezone `Asia/Shanghai`.

## 3. Updating and inspecting

```sh
batch-git schedule update nightly-sync --action pull
batch-git schedule update nightly-sync --cron '0 30 2 * * *'
batch-git schedule update backend-sync --all --overlap queue
batch-git schedule update nightly-sync --disable
batch-git schedule update nightly-sync --enable

batch-git schedule list
batch-git schedule list --registered
batch-git schedule list --json
batch-git schedule plan nightly-sync --json
```

`plan` resolves the declaration and displays the effective repository scope without synchronizing.

Do not mix the global Git-operation preview `--plan` with `schedule plan`. Schedule has its own plan,
doctor, generate, and register/unregister `--dry-run` flows, so `batch-git --plan schedule …` is
rejected.

## 4. Trigger syntax

### Daily time

`--at HH:MM` uses the effective schedule timezone and accepts `00:00` through `23:59`. systemd places
`BATCH_GIT_TZ` in `OnCalendar`. launchd and Windows Task Scheduler triggers continue to use the
system timezone, while the task process receives the corresponding `TZ` environment value.

### Fixed interval

`--every` is a positive integer plus one unit: `s`, `m`, `h`, or `d`, for example `30m`, `6h`, or
`1d`.

### Cron

The format has six fields:

```text
second minute hour day-of-month month day-of-week
```

Supported syntax includes `*`, `?` in day fields, lists `,`, ranges `-`, steps `/`, month names
`JAN` through `DEC`, and weekday names `SUN` through `SAT`. Both numeric `0` and `7` mean Sunday.

```text
0 0 2 * * *              every day at 02:00:00
0 */15 9-17 ? * MON-FRI  every 15 minutes on weekdays from 09:00 through 17:59
30 0 8 1 JAN,JUL *       January 1 and July 1 at 08:00:30
```

To preserve cross-platform semantics, day-of-month and day-of-week cannot both be restricted.
Quartz extensions `L`, `W`, `#`, and the year field are unsupported. launchd calendars do not support
seconds, so the cron seconds field must be exactly `0` for launchd. The first Windows Task Scheduler
implementation does not support cron; use `--at` or `--every` for Windows.

Windows Task Scheduler fixed intervals range from `1m` through `31d`. Values outside that range fail
during `doctor`, `generate`, or `register`.

## 5. Validation, generation, and registration

```sh
batch-git schedule doctor nightly-sync
batch-git schedule doctor --platform launchd --json
batch-git schedule generate nightly-sync --platform launchd
batch-git schedule generate nightly-sync --platform windows
batch-git schedule register nightly-sync --dry-run
batch-git schedule register nightly-sync
```

`doctor` validates declarations, repository selection, and target-platform definitions. Without a
name it checks every declaration. `generate` prints the native definition without registering it.

`--platform auto` selects launchd on macOS, systemd on Linux, and Task Scheduler on Windows. An
explicit platform is useful for previews. Use `--migrate` when moving an existing registration to
another scheduler platform. Replacing a colliding native task that batch-git did not record requires
explicit `--force`.

## 6. Running immediately

```sh
batch-git schedule run nightly-sync
```

`run` uses the declaration's action and scope. With `overlap = "skip"`, an already-locked workspace
is skipped; `queue` waits for the lock. A disabled declaration cannot run manually. Enable it first
with `schedule update <name> --enable`.

## 7. Status and logs

```sh
batch-git schedule status nightly-sync
batch-git schedule status nightly-sync --json
```

Important fields:

- `REGISTERED`: whether batch-git local registration state exists.
- `NATIVE LOADED`: whether the native scheduler currently has the task loaded.
- `DEFINITION MATCHES`: whether task files match the current declaration and registration environment.
- `LAST EXIT CODE`: most recent execution exit code.
- `STDOUT` / `STDERR`: log paths when logging is enabled.
- `NATIVE FILES`: native task-definition files.

Background tasks discard stdout/stderr by default. Enable logging and register again:

```sh
BATCH_GIT_SCHEDULE_LOG=true batch-git schedule register nightly-sync
batch-git schedule status nightly-sync
```

Disabling logging also requires registration:

```sh
BATCH_GIT_SCHEDULE_LOG=false batch-git schedule register nightly-sync
```

When logging is enabled, all three scheduler platforms enter through `schedule native-run --log`.
Each `stdout.log` and `stderr.log` therefore contains a `started` record and a final `finished` or
`failed` record with UTC `started_at` / `finished_at`, `duration_ms`, the schedule action and jobs,
and the child exit code. The child command's normal output remains between those records.

The variable is read during `generate`/`register` and embedded in the native definition. Manual
`schedule run` always produces normal output and ignores this setting. Registered native tasks enter
through the hidden `schedule native-run` action. Regardless of the outer output mode, it forces
`--non-interactive` on the actual synchronization child to prevent Git prompts in a terminal-less
environment; configure non-interactive credentials in advance. If that entry point is invoked with
global `--apply`, the child rechecks the revision after acquiring the workspace lock. Drift found at
that point still returns `stale_workspace_revision`.

## 8. Unregistering and removing

Remove the native task but keep the declaration:

```sh
batch-git schedule unregister nightly-sync
batch-git schedule unregister nightly-sync --dry-run
batch-git schedule unregister nightly-sync --purge-history
```

Remove an unregistered declaration:

```sh
batch-git schedule remove nightly-sync
```

Unregister and remove the declaration together:

```sh
batch-git schedule remove nightly-sync --unregister
batch-git schedule remove nightly-sync --unregister --purge-history
```

Registered declarations cannot be deleted directly by default, preventing an unmanaged native task
from being left behind.

## 9. Platform troubleshooting

Get the actual task ID and file paths from `schedule status` first.

macOS:

```sh
plutil -lint "$HOME/Library/LaunchAgents/<TASK-ID>.plist"
launchctl print "gui/$(id -u)/<TASK-ID>"
launchctl kickstart "gui/$(id -u)/<TASK-ID>"
```

Linux:

```sh
systemctl --user status '<TASK-ID>.timer'
systemctl --user status '<TASK-ID>.service'
systemctl --user list-timers
```

Windows:

```powershell
schtasks.exe /Query /TN '<TASK-ID>' /V /FO LIST
schtasks.exe /Run /TN '<TASK-ID>'
```

Windows native-definition XML lives under `tasks/windows` in the batch-git state directory. Tasks
run as the current interactive user. When schedule logging is enabled, batch-git's internal launcher
appends execution-boundary records and child output to `stdout.log` and `stderr.log` in the state directory.

A useful troubleshooting order is: confirm the declaration is enabled, run `doctor`, register again,
check that `NATIVE LOADED` is yes, check that `DEFINITION MATCHES` is yes, and then inspect the exit
code and logs.
