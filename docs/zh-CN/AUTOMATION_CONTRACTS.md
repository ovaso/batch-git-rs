# 自动化契约

[English](../AUTOMATION_CONTRACTS.md)

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
`add` 的 filter 以及 `commit` 的 hook、签名程序也属于可能继续存活的后代；commit 可能已经
更新本地 ref 后才在后续步骤中超时，因此自动化收到 `timeout` 后必须重新检查 HEAD 和 index，
不能假定该仓库未产生提交或直接重试。
被捕获的 Git stdout 和 stderr 各自最多保留 1 MiB。超过上限时 batch-git 仍会持续排空 pipe，
但只保留输出开头和结尾，并在中间插入 `[batch-git: output truncated]`。机器 receipt 不包含原始
Git 输出；文本详细模式或调试 per-repository 结果的消费者不得假定输出完整。
已注册的原生 schedule 通过隐藏的 `schedule native-run` 入口启动时，也会无条件向其实际
`schedule run` child 传递 `--non-interactive`，不支持依赖终端提示的认证流程。注册时启用
schedule 日志后，每个日志流还会写入面向人工诊断的 `started` 以及最终 `finished` / `failed`
边界记录，包含 UTC 时间、耗时、动作、并发数和退出码。这些文本日志不是机器协议；自动化仍必须
以 JSON receipt 和进程退出码为准。

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
| `workspace` | 已解析的工作区路径及当前 `batchspace.toml` SHA-256 revision；不适用时省略。 |
| `exit_code` / `ok` | 与进程退出码一致。部分仓库失败时 `ok=false`、`exit_code=1`，但 `error=null`。 |
| `data` | 命令结果；允许按命令新增字段。不要假设所有命令具有相同数据模型。 |
| `error` | 仅顶层错误（退出码 `2`）使用；成功或部分仓库失败时为 `null`。 |

批量操作的 `data.results[]` 以清单顺序稳定输出。每项含 `repository`、`directory`、
`status`（`ok`、`skipped`、`failed`）、`detail`、可选 `reason_code`、可选 Git
`exit_code` 及 `synchronized`。`detail` 面向人类，不是稳定决策输入；应优先读取
`status` 和 `reason_code`。

常见仓库级 reason code 包括 `dirty_worktree`、`no_upstream`、`branch_ambiguous`、
`branch_missing`、`repository_unavailable`、`nothing_to_push`、`nothing_to_stage`、
`nothing_to_unstage`、`nothing_to_commit`、`unresolved_conflicts`、`detached_head`、
`repository_operation_in_progress`、`timeout`、`git_exit` 和 `operation_failed`。不同命令可
增加新的 code。

暂存与提交命令使用以下稳定分类：

| code | status | 含义 / 建议 |
|---|---|---|
| `nothing_to_stage` | `skipped` | `add` 没有发现可暂存的非忽略工作树变更。 |
| `nothing_to_unstage` | `skipped` | `unstage` 的 index 已与 HEAD 一致；unborn 仓库的 index 为空。 |
| `nothing_to_commit` | `skipped` | `commit` 没有发现已暂存内容；未暂存和 untracked 内容不会被隐式加入。 |
| `unresolved_conflicts` | `failed` | `add` / `commit` 拒绝未解决冲突；进入对应仓库显式处理。 |
| `detached_head` | `failed` | `commit` 拒绝在 detached HEAD 上创建提交；先 checkout 本地分支。 |
| `repository_operation_in_progress` | `failed` | merge、rebase、cherry-pick、revert 等 operation 正在进行；使用显式 Git continue/abort 流程。 |

## 顶层错误

参数、环境、清单、选择器、锁或持久化错误返回退出码 `2`，并在 `error` 中提供稳定 code：

| code | 含义 / 建议 |
|---|---|
| `invalid_arguments` | 修正 CLI 参数；例如 `commit -m` 的消息不能为空。 |
| `workspace_not_found` | 进入工作区、创建清单或设置绝对 `BATCH_GIT_WORKSPACE`。 |
| `workspace_manifest_invalid` | 修复 `batchspace.toml`。 |
| `unknown_repository` / `ambiguous_repository` / `selector_no_match` | 先用 `list --output json` 获取规范名称。 |
| `stale_workspace_revision` | 重新 plan，使用返回的新 revision。 |
| `workspace_locked` | 等当前任务结束后重试。 |
| `timeout` | 检查连接或以更大的 `--timeout` 重试。 |
| `schedule_invalid` | 检查 schedule 声明和平台限制。 |
| `operation_failed` | 未能进一步稳定分类的顶层失败。 |

这些 code 由类型化错误来源决定，不从 `message` 中搜索英文关键词。错误还包含可演进的
`message`、`retryable` 和可选 `hint`。不要通过匹配 `message` 文字
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
`add`、`commit` 和 `unstage` 也是批量仓库命令，会产生相同的 started / repository_finished /
finished 生命周期；无内容可操作的仓库以 `skipped` 事件出现。

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

`add`、`commit` 和 `unstage` plan 还包含 `parameters`，其首版固定形状如下：

| command | `risk` | `side_effects` | `parameters` |
|---|---|---|---|
| `add` | `index` | `["git_indexes"]` | `{"scope":"all_working_tree_changes","includes":["additions","modifications","deletions"],"force_ignored":false}` |
| `commit` | `local_history` | `["git_objects","local_refs","git_indexes","hooks"]` | `{"message":"…","stages_content":false}` |
| `unstage` | `index` | `["git_indexes"]` | `{"scope":"all_staged_changes","preserves_working_tree":true,"moves_head":false}` |

这三个命令复用标准仓库选择器并在省略选择条件时覆盖整个工作区；首版不接受文件 pathspec。
commit plan 会原样包含调用方提供的消息，日志系统应按普通提交元数据保护它，不要把 plan 当作
秘密存储。plan 不执行 add、hook、签名或 `git read-tree`。

`merge` plan 的每个 `selection.repositories[]` 还包含 `source_branch`、`source_mode`、
`remote_fallback` 和 `source_refresh_remote`。`source_mode` 为 `explicit`、`feature_environment` 或
`workspace_default`；最后一种模式按仓库读取清单中的 `default_branch`，因此不同仓库可以显示
不同来源。`remote_fallback` 只是本地同名分支不存在时的远端解析范围，不表示 plan 已读取或
锁定该远端引用；`source_refresh_remote` 仅在有效的来源刷新开启时出现（显式
`--refresh-source` / `--rs`，或 `merge --default` 的默认设置），表示 apply 会 fetch 后合并的远端。
普通 merge 的 `parameters` 包含 `update_current` 与 `refresh_source` 布尔值；
前者对应 `--update-current` / `--uc` 的 ff-only 目标分支更新，后者对应 `--refresh-source` / `--rs`
的来源刷新。`merge --default` 使用各仓库的 `primary_remote`，普通 merge 未指定远端时刷新同样
使用 `primary_remote`。对 `workspace_default`，apply 的 workspace revision 前置条件会防止清单中的
分支或主远端在 plan 后静默漂移；显式参数和环境变量仍须由调用方在 apply 时保持一致。
`BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE` 只为 `merge --default` 提供来源刷新的默认值（默认 true）；
`BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT` 只为 `merge --feature` 提供当前分支更新的默认值（默认 false）。
CLI 的启用或 `--no-*` 关闭选项优先，plan 中的两个布尔参数始终反映实际 apply 将采用的有效值。
当 `update_current` 为 true 而当前分支没有 upstream 时，该仓库会跳过 ff-only pull 并继续来源合并；
该参数表示请求的行为，不保证每个仓库都实际启动 pull。

apply 在获取工作区锁后、执行 Git 前重新核对 `batchspace.toml` 字节摘要。它防止清单在
plan 与执行之间漂移，但**不**保留远端、HEAD、index 或工作树状态；Git 的正常安全检查仍在
执行时进行。批量操作始终是逐仓库完成，不提供跨仓库事务回滚。commit 的部分仓库成功不会因
其他仓库的 hook、签名、冲突或 Git 失败而自动 reset、amend 或 rebase。

`--apply` 必须同时携带 `--expect-workspace-revision`，且只适用于有副作用的操作。首次
创建工作区时没有可核对的清单 revision，应在明确确认后直接执行。schedule 使用已有的
`schedule plan`、`doctor`、`generate` 和各自的 `--dry-run`；全局 `--plan schedule …`
会明确拒绝，避免混淆两套语义。

`restore` 有意以调用时的当前目录作为工作区根目录，而不是向上查找父清单。因此对它执行
plan 或 apply 时，都必须先 `cd` 到含有 `batchspace.toml` 的工作区根目录。
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
`commands` 是当前构建的实际命令面：默认构建包含 `schedule`；使用 `--no-default-features`
编译的精简二进制不会列出或接受该命令。调用方不得仅根据版本号推断 feature。
支持本组命令的二进制会在 `commands` 中列出 `add`、`commit`、`env`、`unstage`，并在
`safety` 中声明 `commit_stages_content=false`、`add_rejects_unresolved_conflicts=true`、
`commit_rejects_repository_operations=true` 和 `unstage_preserves_working_trees=true`。
`schema operation-result` 返回 v1 envelope 的 JSON Schema；`schema workspace` 返回
`batchspace.toml` 输入的 JSON 表示 schema，而不是序列化后补齐默认值的专用格式。因此它接受
省略的默认字段，例如空的 `repositories` / `schedules` 集合，以及 schedule 的 `enabled`、
`action`、`timezone` 和 `overlap`。Schema 使用 `additionalProperties: true`，因此消费者应验证
核心字段但允许未来新增字段。

规范 schema 标识分别为
`https://github.com/livenv/batch-git/schemas/operation-result-v1.json` 和
`https://github.com/livenv/batch-git/schemas/workspace-v1.json`。这些旧命名空间值是稳定的
协议标识而非仓库下载链接，因此仓库迁移后也保持不变；实际当前文档应通过 `schema` 命令获取。

## 命令覆盖与旧 JSON

所有公开命令（包括 `env list` / `env ls`、`scan`、`clone`、`restore`、`add`、`commit`、
`unstage`、`fetch`、
`sync`、`pull`、`push`、`checkout`、`merge`、`exec`、透传、`forget` 和所有 schedule 子命令）
支持全局 JSON receipt
与 JSONL。`status`、`branch` 也提供旧的 `--json` 直接 payload。对 `exec` 和顶层
`-- <git args>`，batch-git 只提供结构化外壳，风险标记为 `unclassified`；它不会尝试把任意
Git 参数判断为安全或无副作用。

`env list` 不要求工作区。其 receipt 的 `command` 固定为 `env list`，`data.variables` 按帮助文档
顺序列出环境变量；每项包含字符串字段 `name`、`default` 和 `current`。只有使用 `-d` /
`--description` 时才增加字符串字段 `description`。
`current` 是本次调用的最终生效值，因此会包含全局 `--jobs` 覆盖、自动发现的工作区和展开后的
平台 state 目录，也可能包含本机绝对路径。非法环境值仍返回退出码 `2`，不会在清单中混入一个
看似有效的回退值。自动化必须读取这些字段，不应解析文本表格或颜色。

旧 JSON 形状：

| 命令 | 旧 `--json` 顶层形状 |
|---|---|
| `list`、`find`、`info`、`status`、`branch` | object |
| `schedule list`、`schedule doctor` | array |
| `schedule plan`、`schedule status` | object |

## Agent 安全流程

1. 使用 `capabilities`，再使用 `info` 或 `list --output json` 确认工作区和选择器。
2. 写操作前后用 `status --output json`、`branch --output json` 或 `find --output json` 盘点；
   commit 前还应按精确仓库审阅 `git diff --cached`。
3. 对有副作用的 Git/清单操作先 `--plan`；对 push / schedule 使用各自 `--dry-run`。
4. 以每仓库 `status`、`reason_code` 和退出码分别报告 success、skipped 与 failed。
5. commit 只提交 index；不要把 `add` 与 `commit` 合并成未经审阅的一步。未经明确授权，不要
   实际 push、merge、注册/反注册调度器，或通过透传绕过安全边界；使用 `merge --default` 时应
   逐仓库审阅 plan 中的 `source_branch`。
