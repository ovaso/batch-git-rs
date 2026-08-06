# 定时同步手册

[English](../SCHEDULES.md)

batch-git 可以把清单中的 schedule 注册为 macOS `launchd`、Linux
`systemd --user` 或 Windows Task Scheduler 任务。建议优先使用
`action = "sync"`：它只恢复缺失仓库并更新远端引用，不修改已有仓库的工作树。

## 1. 标准流程

```sh
batch-git schedule add nightly-sync --at 02:30 --all
batch-git schedule plan nightly-sync
batch-git schedule doctor nightly-sync
batch-git schedule register nightly-sync
batch-git schedule status nightly-sync
```

修改声明后，需要再次注册才能更新原生任务：

```sh
batch-git schedule update nightly-sync --at 03:00
batch-git schedule register nightly-sync
```

`register` 是幂等 upsert：配置未变时返回 `unchanged`，配置变化时更新同一个任务。

面向 CI 或 agent 时，对任一 schedule 子命令使用全局协议，而非解析提示文字：

```sh
batch-git --output json schedule plan nightly-sync
batch-git --output json schedule register nightly-sync --dry-run
batch-git --output jsonl schedule run nightly-sync
```

新版 receipt 的 `command` 为 `schedule <action>`，并将旧 `schedule list --json` 和
`schedule doctor --json` 的数组直接放在 `data` 中。原有子命令 `--json` 继续保持原形状。
`--output json` 会静默原生 `launchctl`、`systemctl`、`schtasks.exe` 的输出，避免污染协议。

## 2. 创建声明

每天本地时间执行：

```sh
batch-git schedule add nightly-sync --at 02:30 --all
```

固定间隔执行：

```sh
batch-git schedule add backend-sync \
  --every 30m \
  --repo service-api \
  --repo service-worker
```

六段式 cron：

```sh
batch-git schedule add business-hours \
  --action pull \
  --cron '0 */15 9-17 ? * MON-FRI' \
  --repo service-api
```

`add` 必须从 `--at`、`--every`、`--cron` 中选择一个，并从 `--all`、可重复的
`--repo` 中选择一种范围。仓库相对目录会在写入时规范化为仓库名称。

关键选项：

- `--action sync|pull`：默认 `sync`；
- `--overlap skip|queue`：已有工作区操作时跳过或等待，默认 `skip`；
- `--disabled`：创建禁用声明；禁用状态下不能 plan、run 或 register。

schedule 时区通过项目专属环境变量 `BATCH_GIT_TZ` 设置，例如：

```sh
BATCH_GIT_TZ=Asia/Shanghai batch-git schedule register nightly-sync
```

未设置或设置为空时，不注入时区覆盖，使用操作系统时区。修改该变量后需要重新
执行 `schedule register`，使原生定义和注册摘要同步更新。值应使用操作系统支持的
无空白时区标识，例如 IANA 时区 `Asia/Shanghai`。

## 3. 修改和查看

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

`plan` 只解析声明并展示实际仓库范围，不执行同步。

不要将全局 Git 操作预览 `--plan` 与 `schedule plan` 混用。schedule 已有自己的 plan、doctor、
generate 与 register/unregister `--dry-run`，因此 `batch-git --plan schedule …` 会拒绝执行。

## 4. 触发器语法

### 每日时间

`--at HH:MM` 使用有效 schedule 时区，范围为 `00:00` 到 `23:59`。systemd 会将
`BATCH_GIT_TZ` 写入 `OnCalendar`；launchd 和 Windows Task Scheduler 的触发器
仍使用系统时区，但任务进程会收到对应的 `TZ` 环境。

### 固定间隔

`--every` 是正整数加一个单位：`s`、`m`、`h`、`d`，例如 `30m`、`6h`、`1d`。

### Cron

格式为六段式：

```text
秒 分 时 日 月 周
```

支持 `*`、日/周字段中的 `?`、列表 `,`、范围 `-`、步长 `/`、月份缩写
`JAN` 到 `DEC`、星期缩写 `SUN` 到 `SAT`。数字 `0` 和 `7` 都表示星期日。

```text
0 0 2 * * *              每天 02:00:00
0 */15 9-17 ? * MON-FRI  工作日 09:00-17:59，每 15 分钟
30 0 8 1 JAN,JUL *       每年 1 月和 7 月 1 日 08:00:30
```

为保持跨平台语义一致，不允许同时限制“日”和“周”。不支持 Quartz 扩展 `L`、
`W`、`#` 和年份字段。launchd 日历不支持秒，因此选择 launchd 时 cron 秒字段
必须严格为 `0`。Windows Task Scheduler 首版不支持 cron；选择 Windows 平台时
使用 cron 的声明会直接报错，应改用 `--at` 或 `--every`。

Windows Task Scheduler 的固定间隔最短为 `1m`、最长为 `31d`。超出该范围的
`--every` 声明在 `doctor`、`generate` 或 `register` 时会报错。

## 5. 验证、生成和注册

```sh
batch-git schedule doctor nightly-sync
batch-git schedule doctor --platform launchd --json
batch-git schedule generate nightly-sync --platform launchd
batch-git schedule generate nightly-sync --platform windows
batch-git schedule register nightly-sync --dry-run
batch-git schedule register nightly-sync
```

`doctor` 检查声明、仓库选择和目标平台定义；不传名称时检查全部声明。
`generate` 只把原生定义输出到终端，不注册。

`--platform auto` 在 macOS 选择 launchd，在 Linux 选择 systemd，在 Windows
选择 Task Scheduler。显式平台可用于预览。移动既有注册平台时使用 `--migrate`；
替换一个未被 batch-git 登记、但任务 ID 冲突的原生任务时必须显式使用 `--force`。

## 6. 立即运行

```sh
batch-git schedule run nightly-sync
```

`run` 使用声明中的 action 和范围。`overlap = "skip"` 时，工作区已锁定会跳过；
`queue` 时会等待锁释放。禁用声明不能人工运行，需先执行 `schedule update <name>
--enable`。

## 7. 状态与日志

```sh
batch-git schedule status nightly-sync
batch-git schedule status nightly-sync --json
```

重点字段：

- `REGISTERED`：是否存在 batch-git 本地注册状态；
- `NATIVE LOADED`：原生调度器当前是否加载任务；
- `DEFINITION MATCHES`：任务文件是否与当前声明和注册环境一致；
- `LAST EXIT CODE`：最近一次执行退出码；
- `STDOUT` / `STDERR`：启用日志时的文件路径；
- `NATIVE FILES`：原生任务定义文件。

后台任务默认丢弃 stdout/stderr。开启日志后重新注册：

```sh
BATCH_GIT_SCHEDULE_LOG=true batch-git schedule register nightly-sync
batch-git schedule status nightly-sync
```

关闭日志也需要重新注册：

```sh
BATCH_GIT_SCHEDULE_LOG=false batch-git schedule register nightly-sync
```

该变量在 `generate/register` 时读取并固化到原生定义。手动 `schedule run` 始终
正常输出，不受此变量影响。已注册的原生任务通过隐藏的 `schedule native-run` 入口启动时，
无论外层输出模式都会向实际同步 child 强制传递 `--non-interactive`，避免无终端环境中的
Git 提示；需要预先配置无交互认证方式。若以全局 `--apply` 调用该入口，child 会在获取
工作区锁后再次核对 revision；此时发现的漂移仍作为 `stale_workspace_revision` 返回。

## 8. 反注册和删除

只移除原生任务，保留声明：

```sh
batch-git schedule unregister nightly-sync
batch-git schedule unregister nightly-sync --dry-run
batch-git schedule unregister nightly-sync --purge-history
```

删除未注册声明：

```sh
batch-git schedule remove nightly-sync
```

一次完成反注册和声明删除：

```sh
batch-git schedule remove nightly-sync --unregister
batch-git schedule remove nightly-sync --unregister --purge-history
```

已注册任务默认不能直接删除声明，避免留下失去管理来源的系统任务。

## 9. 平台排查

实际任务 ID 和文件路径先从 `schedule status` 获取。

macOS：

```sh
plutil -lint "$HOME/Library/LaunchAgents/<TASK-ID>.plist"
launchctl print "gui/$(id -u)/<TASK-ID>"
launchctl kickstart "gui/$(id -u)/<TASK-ID>"
```

Linux：

```sh
systemctl --user status '<TASK-ID>.timer'
systemctl --user status '<TASK-ID>.service'
systemctl --user list-timers
```

Windows：

```powershell
schtasks.exe /Query /TN '<TASK-ID>' /V /FO LIST
schtasks.exe /Run /TN '<TASK-ID>'
```

Windows 原生定义 XML 保存在 batch-git state 目录的 `tasks/windows` 子目录。
任务以当前交互用户身份运行；开启 schedule 日志时，由 batch-git 的内部启动器
将输出追加到状态目录中的 `stdout.log` 和 `stderr.log`。

常见判断顺序：声明是否 enabled、`doctor` 是否通过、是否重新 register、
`NATIVE LOADED` 是否为 yes、`DEFINITION MATCHES` 是否为 yes，最后查看退出码和日志。
