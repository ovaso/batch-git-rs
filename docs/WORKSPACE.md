# workspace.toml 与运行时配置

## 1. 清单定位

`workspace.toml` 是工作区的声明式事实载体。普通命令从当前目录向上查找最近的
清单；`BATCH_GIT_WORKSPACE` 可以指定绝对工作区路径。`scan` 和 `restore` 始终以
当前目录为根目录，避免误修改父目录工作区。

## 2. 完整示例

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

## 3. 仓库字段

| 字段 | 必需 | 说明 |
|---|---|---|
| `name` | 是 | 工作区内唯一的仓库名称 |
| `directory` | 是 | 工作区内唯一的相对目录，不允许绝对路径、`.` 或 `..` |
| `default_branch` | 是 | `checkout --default` 的目标分支，也是 `merge --default` 的源分支 |
| `primary_remote` | 是 | 主远端名称，必须存在于 `remotes` 中；缺省按 `origin` 读取 |
| `created_at` | 是 | RFC 3339 时间 |
| `synced_at` | 否 | 最近一次成功同步时间 |
| `remotes` | 是 | 至少一个命名远端 |
| `fetch_url` | 是 | fetch URL |
| `push_url` | 否 | 独立 push URL；省略或等于 `fetch_url` 时清除仓库中的独立 push URL |

清单不保存当前分支、所有本地/远端分支、HEAD、tag 或工作树修改。这些动态数据
始终以仓库 `.git` 为准。

## 4. 写入与安全边界

- 当前 schema 版本固定为 `1`；
- 名称和目录必须唯一；
- 时间必须是合法 RFC 3339；
- HTTP(S) URL 中的用户信息在写入前移除；
- 写入采用同目录临时文件、同步和原子替换；
- 会执行 Git 或修改清单的进程使用 `.workspace.lock`；
- 程序会规范化 TOML 格式，不保证保留手写注释和原始排版。

`schedules` 可以整体省略，等同于空列表。手写 schedule 时也可省略有默认值的 `enabled`、
`action`、`timezone` 和 `overlap`；`schema workspace` 输出的 schema 对应这种输入形式，而不是要求
写入程序序列化时会补齐的默认字段。

当使用 `batch-git --output json --plan …` 时，receipt 中的 `workspace.revision` 是当前
`workspace.toml` 原始字节的 `sha256:<hex>` 摘要。将它只作为紧随其后的
`--apply --expect-workspace-revision` 前置条件，不要写回清单或把它当作 schema 字段。apply
会在持锁后重新计算摘要；注释或空白的任何改动也会使该摘要失效。

因此，注释不应承载业务含义。手工编辑后建议先运行只读命令校验：

```sh
batch-git info
```

## 5. 环境变量

配置优先级为：命令行参数 > 环境变量 > 内建默认值。非法值会报错，不会静默
回退。

| 环境变量 | 默认值 | 说明 |
|---|---:|---|
| `BATCH_GIT_JOBS` | `4` | 最大并发仓库数，必须大于 `0` |
| `BATCH_GIT_SCAN_DEPTH` | `1` | `scan` 默认扫描深度，必须大于 `0` |
| `BATCH_GIT_WORKSPACE` | 未设置 | 普通命令使用的绝对工作区路径 |
| `BATCH_GIT_STATE_DIR` | 平台用户 state 目录 | schedule 注册状态和日志根目录 |
| `BATCH_GIT_SCHEDULE_LOG` | `false` | 注册任务是否记录 stdout/stderr |
| `BATCH_GIT_TZ` | 系统时区 | schedule 任务时区；设置后需重新 register，不允许空白字符 |
| `BATCH_GIT_REMOTE` | 未设置 | 普通 checkout/merge 的远端消歧名称；`--default` 固定使用各仓库的 `primary_remote` |
| `CURRENT_FEATURE_BRANCH` | 未设置 | `checkout --feature` / `cf` 的目标分支，以及 `merge --feature` 的源分支 |
| `BATCH_GIT_MERGE_UPDATE_CURRENT` | `false` | merge 前是否 ff-only 更新当前分支 |
| `BATCH_GIT_MERGE_REFRESH_SOURCE` | `false` | merge 前是否 fetch 并使用最新 remote-tracking 来源分支 |
| `BATCH_GIT_PASSTHROUGH_VERBOSE` | `true` | 全仓库 Git 透传是否展示成功输出 |
| `NO_COLOR` | 未设置 | 非空时禁用颜色 |

布尔变量接受 `1/0`、`true/false`、`yes/no`、`on/off`，不区分大小写。

## 6. 本地状态文件

`.workspace.lock` 位于工作区根目录，用于串行化可能冲突的操作。schedule 的注册
状态和可选日志位于用户 state 目录，不写入共享清单：

- 显式设置：`BATCH_GIT_STATE_DIR`；
- macOS 默认：用户 Library 下的 Application Support 状态目录；
- Windows 默认：`%LOCALAPPDATA%\batch-git`，不可用时回退到用户目录下的
  `AppData\Local\batch-git`；
- Linux/Unix 默认：`$XDG_STATE_HOME/batch-git`，未设置时使用
  `$HOME/.local/state/batch-git`。

实际任务文件和日志路径应以 `batch-git schedule status <name>` 输出为准，不要
在脚本中猜测平台路径。
