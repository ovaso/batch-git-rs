# 自动化契约

本文件定义 `batch-git` 面向 CI、脚本和 agent 的稳定接口。人类表格、颜色和说明文字可
改进；自动化必须使用版本化输出并根据退出码和稳定字段判断结果。除非在新的主版本中声明，
同一主版本只会新增字段，不会删除字段或改变既有字段类型。

## 选择输出协议

新集成一律使用全局输出参数：

```sh
batch-git --output json --request-id task-42 status
batch-git --output jsonl sync --match 'service-*'
```

| 格式 | 适用场景 | stdout 保证 |
|---|---|---|
| `text`（默认） | 人类终端 | 表格、提示和必要的子进程诊断。 |
| `json` | 一次请求/响应式的 agent、CI、脚本 | 恰好一个紧凑 JSON 文档；成功时 stderr 为空。 |
| `jsonl` | 长批量任务和进度消费 | 每行一个独立 JSON 事件；没有表格、进度条或 Git 原始输出。 |

`--output json` / `jsonl` 隐式以非交互方式启动 Git 子进程，避免 TTY 提示污染协议。
需要在文本模式下显式禁止交互时使用 `--non-interactive`。`--timeout 30s`、`5m` 或 `1h`
限制每个由 batch-git 启动的系统 Git 子进程；它不限制工作区锁等待、git2 本地操作或原生
调度器命令。timeout 只终止直接启动的 Git 子进程，不能保证终止它再派生的认证、传输或
helper 进程；收到 timeout 结果后，这些后代进程仍可能存活。
已注册的原生 schedule 通过隐藏的 `schedule native-run` 入口启动时，也会无条件向其实际
`schedule run` child 传递 `--non-interactive`，不支持依赖终端提示的认证流程。

旧的子命令 `--json` 仍保持原有顶层形状，例如 `list --json` 是对象、`schedule list --json`
是数组。它仅用于兼容现有调用；新调用应使用全局 `--output json`。二者同时出现时，
全局 `--output` 决定渲染方式。

## v1 JSON receipt

`--output json` 的顶层结构固定为：

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

核心字段如下：

| 字段 | 含义 |
|---|---|
| `api_version` | 当前固定为 `v1`。 |
| `command` | 实际执行的命令；schedule 形如 `schedule list` / `schedule run`。 |
| `request_id` | 调用方提供的关联标识；未提供则省略。长度为 1–128 个非控制字符。 |
| `workspace` | 已解析的工作区路径及当前 `workspace.toml` SHA-256 revision；不适用时省略。 |
| `exit_code` / `ok` | 与进程退出码一致。部分仓库失败时 `ok=false`、`exit_code=1`，但 `error=null`。 |
| `data` | 命令结果；允许按命令新增字段。不要假设所有命令具有相同数据模型。 |
| `error` | 仅顶层错误（退出码 `2`）使用；成功或部分仓库失败时为 `null`。 |

批量操作的 `data.results[]` 以清单顺序稳定输出。每项含 `repository`、`directory`、
`status`（`ok`、`skipped`、`failed`）、`detail`、可选 `reason_code`、可选 Git
`exit_code` 及 `synchronized`。`detail` 面向人类，不是稳定决策输入；应优先读取
`status` 和 `reason_code`。

常见仓库级 reason code 包括 `dirty_worktree`、`no_upstream`、`branch_ambiguous`、
`branch_missing`、`repository_unavailable`、`nothing_to_push`、`nothing_to_commit`、
`timeout`、`git_exit` 和 `operation_failed`。不同命令可增加新的 code。

## 顶层错误

参数、环境、清单、选择器、锁或持久化错误返回退出码 `2`，并在 `error` 中提供稳定 code：

| code | 含义 / 建议 |
|---|---|
| `invalid_arguments` | 修正 CLI 参数。 |
| `workspace_not_found` | 进入工作区、创建清单或设置绝对 `BATCH_GIT_WORKSPACE`。 |
| `workspace_manifest_invalid` | 修复 `workspace.toml`。 |
| `unknown_repository` / `ambiguous_repository` / `selector_no_match` | 先用 `list --output json` 获取规范名称。 |
| `stale_workspace_revision` | 重新 plan，使用返回的新 revision。 |
| `workspace_locked` | 等当前任务结束后重试。 |
| `timeout` | 检查连接或以更大的 `--timeout` 重试。 |
| `schedule_invalid` | 检查 schedule 声明和平台限制。 |
| `operation_failed` | 未能进一步稳定分类的顶层失败。 |

错误还包含可演进的 `message`、`retryable` 和可选 `hint`。不要通过匹配 `message` 文字
做自动决策。机器协议不会默认包含 Git 的原始 stdout/stderr；HTTP(S) URL 的 user-info
也会在机器诊断中清理。

## JSON Lines 事件

`--output jsonl` 使用同一 `api_version`、`command`、`request_id` 和可用的 `workspace`
字段。短命令至少输出 `started` 和带 `exit_code` / `ok` 的 `finished`；批量仓库命令在
二者之间输出按清单顺序排列的 `repository_finished` 事件：

```json
{"api_version":"v1","event":"started","command":"sync","data":{"repositories":2}}
{"api_version":"v1","event":"repository_finished","command":"sync","data":{"repository_index":0,"result":{"repository":"service-api","status":"ok"}}}
{"api_version":"v1","event":"finished","command":"sync","exit_code":0,"ok":true,"data":{"ok":2,"skipped":0,"failed":0}}
```

无法完成解析或初始化时会输出带 `exit_code=2` 和 `ok=false` 的 `error` 事件。事件消费方应
以最终 `finished`（或 `error`）和进程退出码作为最终结论，不应只根据中间进度事件判定成功。
`started` 在批量仓库工作开始前输出。为保持 v1 的清单顺序保证，较晚索引的仓库即使先完成，
其 `repository_finished` 事件也可能等待所有前序仓库完成后才输出；事件顺序不是完成时间顺序。
单仓库 `clone` 也遵循完整生命周期，在系统 Git 开始前发出 `started`，随后发出索引为 `0` 的
`repository_finished` 和最终 `finished`。若 clone 在登记前失败，仓库事件的 `repository` 使用
请求的目标目录作为稳定标识。

## 计划与 apply

对内建的可写 Git/清单操作可以先做零副作用的预览：

```sh
plan="$(batch-git --output json --plan sync --match 'service-*')"
# 从 plan.workspace.revision 读取 revision
batch-git --output json --apply \
  --expect-workspace-revision 'sha256:…' \
  sync --match 'service-*'
```

plan 的 `data` 包含 `mode=plan`、已解析仓库范围、风险、预期副作用、并发数和
`workspace_revision`。它不会写清单、修改仓库、访问远端或注册调度器。

apply 在获取工作区锁后、执行 Git 前重新核对 `workspace.toml` 字节摘要。它防止清单在
plan 与执行之间漂移，但**不**保留远端状态，也不重验 HEAD / dirty 状态；Git 的正常安全
检查仍在执行时进行。批量操作始终是逐仓库完成，不提供跨仓库事务回滚。

`--apply` 必须同时携带 `--expect-workspace-revision`，且只适用于有副作用的操作。首次
创建工作区时没有可核对的清单 revision，应在明确确认后直接执行。schedule 使用已有的
`schedule plan`、`doctor`、`generate` 和各自的 `--dry-run`；全局 `--plan schedule …`
会明确拒绝，避免混淆两套语义。

`restore` 有意以调用时的当前目录作为工作区根目录，而不是向上查找父清单。因此对它执行
plan 或 apply 时，都必须先 `cd` 到含有 `workspace.toml` 的工作区根目录。
clone 或 restore 的 Git 子进程失败/超时时，目标目录会保留供人工检查，且不会自动登记；
batch-git 不会递归删除该目录，因为其内容可能已被并发进程写入。处理该目录后才能重试。

## 能力与 schema 发现

不要让 agent 假定安装的版本等于仓库源码。先查询当前二进制：

```sh
batch-git --output json capabilities
batch-git schema operation-result
batch-git --output json schema workspace
```

`capabilities` 声明二进制版本、协议版本、输出格式、可用命令、plan/apply 边界和安全属性。
`schema operation-result` 返回 v1 envelope 的 JSON Schema；`schema workspace` 返回
`workspace.toml` 输入的 JSON 表示 schema，而不是序列化后补齐默认值的专用格式。因此它接受
省略的默认字段，例如空的 `repositories` / `schedules` 集合，以及 schedule 的 `enabled`、
`action`、`timezone` 和 `overlap`。Schema 使用 `additionalProperties: true`，因此消费者应验证
核心字段但允许未来新增字段。

## 命令覆盖与旧 JSON

所有公开命令（包括 `scan`、`clone`、`restore`、`fetch`、`sync`、`pull`、`push`、
`checkout`、`merge`、`exec`、透传、`forget` 和所有 schedule 子命令）支持全局 JSON receipt
与 JSONL。`status`、`branch` 也提供旧的 `--json` 直接 payload。对 `exec` 和顶层
`-- <git args>`，batch-git 只提供结构化外壳，风险标记为 `unclassified`；它不会尝试把任意
Git 参数判断为安全或无副作用。

旧 JSON 形状：

| 命令 | 旧 `--json` 顶层形状 |
|---|---|
| `list`、`find`、`info`、`status`、`branch` | object |
| `schedule list`、`schedule doctor` | array |
| `schedule plan`、`schedule status` | object |

## Agent 安全流程

1. 使用 `capabilities`，再使用 `info` 或 `list --output json` 确认工作区和选择器。
2. 写操作前后用 `status --output json`、`branch --output json` 或 `find --output json` 盘点。
3. 对有副作用的 Git/清单操作先 `--plan`；对 push / schedule 使用各自 `--dry-run`。
4. 以每仓库 `status`、`reason_code` 和退出码分别报告 success、skipped 与 failed。
5. 未经明确授权，不要实际 push、merge、注册/反注册调度器，或通过透传绕过安全边界。
