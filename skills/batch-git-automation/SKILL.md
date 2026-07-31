---
name: batch-git-automation
description: 在 batch-git 多 Git 仓库工作区中安全、可审计地进行自动化开发与维护。用于发现 batch-git 能力、读取 workspace.toml 工作区状态、使用 JSON receipt 或 JSONL 编排批量 Git 操作、先 plan 再 apply、协调跨仓库分支/同步/推送，或管理定时同步；当用户提及 batch-git、多仓库 Git、workspace.toml、批量同步、agent/CI 自动化时使用。
---

# Batch Git Automation

通过 `batch-git` CLI 管理由 `workspace.toml` 声明的多个独立 Git 仓库。将清单视为恢复和
远端配置的事实来源；将各仓库 Git 状态视为实时事实来源。不要安装此 skill、启动守护进程或
引入 MCP：本 skill 只编排现有 CLI。

先阅读[能力名册](references/capability-roster.md)。需要字段、事件或兼容性细节时，再阅读
仓库中的[自动化契约](../../docs/AUTOMATION_CONTRACTS.md)。代码版本与用户安装的二进制可能
不同，因此不要仅依据此文件猜测参数或协议。

## 默认流程

1. 在目标工作区执行 `batch-git --output json capabilities`，确认当前二进制的协议版本、命令和
   plan/apply 限制。
2. 用 `batch-git --output json info` 和 `batch-git --output json list` 确认工作区根目录与规范
   仓库名。路径存在歧义时设置绝对 `BATCH_GIT_WORKSPACE`。
3. 在任何本地、远端或调度器写入前后，用 `status --output json`、`branch --output json`，必要时
   `find '<pattern>' --output json` 盘点。分别记录 `ok`、`skipped` 和 `failed`。
4. 对可写的 Git/清单操作先调用 `--output json --plan`，核对 `data.selection`、风险、预期副作用
   和 `workspace.revision`。plan 不访问远端，也不是跨仓库事务。
5. 只有用户明确授权实际变更时，才以同一精确命令加上
   `--apply --expect-workspace-revision <plan revision>` 执行。清单变更后必须重新 plan。
6. 用新的 JSON receipt 和退出码报告结果；不要解析表格、颜色、`detail` 自然语言或 Git 原始输出。

对首次创建工作区的 `scan` / `clone`，可能没有可供 apply 核对的清单 revision。展示 plan 后，
请求明确授权再直接执行；不要伪造 revision。

`restore` 特意只把调用时当前目录视为工作区根目录；对它 plan 或 apply 前，必须先进入含有
`workspace.toml` 的根目录，不能从子仓库目录调用。

## 输出纪律

新自动化默认使用全局 `--output json`：它在 stdout 产生恰好一个 v1 receipt，顶层错误也有
稳定 `error.code`。可选 `--request-id <opaque-id>` 将关联标识回显到 receipt 和 JSONL 事件。

```sh
batch-git --output json --request-id change-184 sync --match 'service-*'
batch-git --output json --plan pull service-api
batch-git --output json --apply --expect-workspace-revision 'sha256:…' pull service-api
```

- 对 `clone`、`restore`、`fetch`、`sync`、`pull` 等长任务，可使用 `--output jsonl`。逐行读取
  `started`、`repository_finished`、`finished`，以最终事件和进程退出码为结论。clone 失败时，
  单个仓库事件以请求的目标目录作为稳定标识。
  `started` 在仓库工作前出现；仓库事件按清单顺序而非完成时间输出：较晚的仓库可能已完成，
  但会等待前序仓库事件后才出现。
- 旧的子命令 `--json` 只用于兼容原有脚本。不要把它与新 receipt 混为同一 schema；同时提供时
  全局 `--output` 优先。
- 机器模式隐式禁用 Git 终端交互。文本模式下无人值守时使用 `--non-interactive`；必要时加
  `--timeout 30s`、`5m` 或 `1h`。timeout 仅终止直接启动的系统 Git 子进程，不约束锁等待，
  也不能保证结束其认证、传输或 helper 后代进程。
- 使用 `status`、`reason_code`、`error.code` 和退出码决策。`detail` 与 `error.message` 仅用于
  人类诊断，未来可演进。

## 选择与安全边界

精确名称或 workspace 相对目录是首选选择器；`--match` 与 `find --repo` 只支持区分大小写的
`*` 通配符。先从 `list --output json` 复制规范名称，避免未验证的广泛 `--match '*'`。

优先安全梯度：只读盘点 → `fetch` → `sync` → `pull` → `checkout` / `merge` / `push`。`sync`
适合无人值守：只恢复缺失仓库并更新远端引用，不改已有工作树。`pull` 只能 fast-forward，
不会 stash、rebase、reset 或解决冲突。

clone 或 restore 失败/超时时，目标目录会保留供人工检查，不能假定重试会覆盖或清理它；
先确认内容并明确处理该目录，随后才能再次使用同一目标。

`exec` 和顶层 `batch-git -- <git-args...>` 只能提供结构化结果外壳，风险为 `unclassified`。
对这些逃生舱，默认只使用低风险只读 Git 命令；破坏性、历史改写或远端写入命令必须有用户针对
精确仓库、精确参数和影响范围的明确授权。全局选项必须放在透传分隔符之前。

## 远端、分支与定时任务

- 实际 `push` 前先运行 `push --dry-run`；只有明确授权时使用 `--set-upstream`，绝不 force push。
- `checkout` 前检查工作树与分支；远端同名分支歧义必须传 `--remote`。`merge` 需要明确的源分支、
  范围和授权；不要自动解决、abort 或继续冲突。
- schedule 使用其专用流程：`schedule plan` → `doctor` → `generate` 或 `register --dry-run` →
  明确授权后的 `register`。不要使用全局 `--plan schedule …`，该组合会被拒绝以避免语义混淆。
  `register`、`unregister`、`remove --unregister` 会改变原生调度器，必须单独得到授权。
- 已注册 schedule 的隐藏 `native-run` child 始终携带 `--non-interactive`；在注册前配置可用的
  非交互凭据，不能依赖 Git 终端提示。

## 报告

结束时报告工作区、选择条件、精确命令、receipt 的请求 ID（如有）、每个仓库的状态和 reason
code、最终退出码，以及需要人工处理的事项。不要在报告、命令或结构化数据中复制凭据、私有
remote URL user-info 或 Git 子进程原始输出。
