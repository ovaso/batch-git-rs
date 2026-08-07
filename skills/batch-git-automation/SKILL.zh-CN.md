---
name: batch-git-automation
description: 在 batch-git 多 Git 仓库工作区中安全、可审计地进行自动化开发与维护。用于发现 batch-git 能力、读取 batchspace.toml 工作区状态、使用 JSON receipt 或 JSONL 编排批量暂存、提交和 Git 操作、先 plan 再 apply、协调跨仓库分支/同步/推送，或管理定时同步；当用户提及 batch-git、多仓库 Git、batchspace.toml、批量同步、agent/CI 自动化时使用。
---

# Batch Git 自动化

[English](SKILL.md)

通过 `batch-git` CLI 管理由 `batchspace.toml` 声明的多个独立 Git 仓库。将清单视为恢复和
远端配置的事实来源；将各仓库 Git 状态视为实时事实来源。不要安装此 skill、启动守护进程或
引入 MCP：本 skill 只编排现有 CLI。

先阅读[能力名册](references/zh-CN/capability-roster.md)。需要字段、事件或兼容性细节时，再阅读
仓库中的[自动化契约](../../docs/zh-CN/AUTOMATION_CONTRACTS.md)。代码版本与用户安装的二进制可能
不同，因此不要仅依据此文件猜测参数或协议。

## 默认流程

1. 在目标工作区执行 `batch-git --output json capabilities`，确认当前二进制的协议版本、命令和
   plan/apply 限制。需要诊断运行配置时，再执行 `batch-git --output json env list` 读取环境变量
   的最终生效值，不要解析彩色文本表格。`commands` 反映当前构建 features；未列出 `schedule`
   时不得尝试调度流程。
2. 用 `batch-git --output json info` 和 `batch-git --output json list` 确认工作区根目录与规范
   仓库名。路径存在歧义时设置绝对 `BATCH_GIT_WORKSPACE`。
3. 在任何本地、远端或调度器写入前后，用 `status --output json`、`branch --output json`，必要时
   `find '<pattern>' --output json` 盘点。commit 前还要对精确仓库审阅 `git diff --cached`。
   分别记录 `ok`、`skipped` 和 `failed`。
4. 对可写的 Git/清单操作先调用 `--output json --plan`，核对 `data.selection`、风险、预期副作用
   和 `workspace.revision`。merge plan 还应逐仓库核对 `source_branch`、`source_mode` 和
   `remote_fallback` 与 `source_refresh_remote`。plan 不访问远端，也不是跨仓库事务。
5. 只有用户明确授权实际变更时，才以同一精确命令加上
   `--apply --expect-workspace-revision <plan revision>` 执行。清单变更后必须重新 plan。
6. 用新的 JSON receipt 和退出码报告结果；不要解析表格、颜色、`detail` 自然语言或 Git 原始输出。

对首次创建工作区的 `scan` / `clone`，可能没有可供 apply 核对的清单 revision。展示 plan 后，
请求明确授权再直接执行；不要伪造 revision。

`restore` 特意只把调用时当前目录视为工作区根目录；对它 plan 或 apply 前，必须先进入含有
`batchspace.toml` 的根目录，不能从子仓库目录调用。

## 输出纪律

新自动化默认使用全局 `--output json`：它在 stdout 产生恰好一个 v1 receipt，顶层错误也有
稳定 `error.code`。可选 `--request-id <opaque-id>` 将关联标识回显到 receipt 和 JSONL 事件。

```sh
batch-git --output json --request-id change-184 sync --match 'service-*'
batch-git --output json --plan pull service-api
batch-git --output json --apply --expect-workspace-revision 'sha256:…' pull service-api
```

- 对 `clone`、`restore`、`add`、`commit`、`unstage`、`fetch`、`sync`、`pull` 等批量或长任务，
  可使用 `--output jsonl`。逐行读取
  `started`、`repository_finished`、`finished`，以最终事件和进程退出码为结论。clone 失败时，
  单个仓库事件以请求的目标目录作为稳定标识。
  `started` 在仓库工作前出现；仓库事件按清单顺序而非完成时间输出：较晚的仓库可能已完成，
  但会等待前序仓库事件后才出现。
- 旧的子命令 `--json` 只用于兼容原有脚本。不要把它与新 receipt 混为同一 schema；同时提供时
  全局 `--output` 优先。
- 机器模式隐式禁用 Git 终端交互。文本模式下无人值守时使用 `--non-interactive`；必要时加
  `--timeout 30s`、`5m` 或 `1h`。timeout 仅终止直接启动的系统 Git 子进程，不约束锁等待，
  也不能保证结束其认证、传输、filter、hook 或签名后代进程。commit timeout 后本地 ref 可能
  已经更新，必须重新检查 HEAD 和 index，不能盲目重试。
- 使用 `status`、`reason_code`、`error.code` 和退出码决策。`detail` 与 `error.message` 仅用于
  人类诊断，未来可演进；顶层 code 来自类型化错误，不要从 message 关键词自行重分类。
- 捕获的 Git stdout/stderr 可能在 1 MiB 后保留头尾并标记截断。机器 receipt 本身不携带原始
  Git 输出；需要完整日志时，应在精确仓库中使用用户明确授权的专用日志方案。
- 已注册任务设置 `BATCH_GIT_SCHEDULE_LOG=true` 时，两个日志流都会写入面向人工诊断的边界信息：带数值
  UTC 偏移的本地 RFC 3339 开始/结束时间、耗时、动作、并发数和退出码。它们是可演进的文本诊断，不是协议；仍以 JSON receipt
  和进程退出码作出自动化决策。

## 选择与安全边界

精确名称或 workspace 相对目录是首选选择器；`--match` 与 `find --repo` 只支持区分大小写的
`*` 通配符。先从 `list --output json` 复制规范名称，避免未验证的广泛 `--match '*'`。

优先安全梯度：只读盘点 → `fetch` → `sync` → `add` / `unstage` → 审阅 staged diff → `commit` →
`pull` → `checkout` / `merge` / `push`。`sync`
适合无人值守：只恢复缺失仓库并更新远端引用，不改已有工作树。`pull` 只能 fast-forward，
不会 stash、rebase、reset 或解决冲突。

clone 或 restore 失败/超时时，目标目录会保留供人工检查，不能假定重试会覆盖或清理它；
先确认内容并明确处理该目录，随后才能再次使用同一目标。
清单仓库目录的真实路径必须保持在工作区内；不要通过 symlink 把仓库或 clone/restore 目标指向
工作区外。遇到 `workspace_manifest_invalid` 时先检查相对路径和符号链接边界。

`exec` 和顶层 `batch-git -- <git-args...>` 只能提供结构化结果外壳，风险为 `unclassified`。
对这些逃生舱，默认只使用低风险只读 Git 命令；破坏性、历史改写或远端写入命令必须有用户针对
精确仓库、精确参数和影响范围的明确授权。全局选项必须放在透传分隔符之前。

## 暂存与本地提交

- `add`、`commit`、`unstage` 接受规范仓库名、workspace 相对目录、可重复 `--match` 或
  `--all`；省略选择器时默认整个工作区。首版不接受文件 pathspec。
- `add` 暂存全部非忽略的新增、修改和删除，不 force 加入 ignored 文件，并拒绝未解决冲突。
  `nothing_to_stage` 是正常跳过；需要按文件处理冲突时，只能在精确仓库中使用明确的原生 Git。
- add 后先逐仓库审阅 `git diff --cached`，再运行 `commit -m <message>`。commit 只提交 index，
  不 implicit add，不 amend，不创建空提交，也不绕过 hook；`nothing_to_commit` 是正常跳过。
- commit 拒绝 detached HEAD、未解决冲突，以及进行中的 merge、rebase、cherry-pick、revert 等
  Git operation。分别按 `detached_head`、`unresolved_conflicts` 和
  `repository_operation_in_progress` 处理，不要用批量命令自动 continue 或 abort。
- `unstage` 全量恢复 index，但保留全部工作树文件且不移动 HEAD。unborn HEAD 使用
  `git read-tree --empty`；`nothing_to_unstage` 是正常跳过。它不能撤销已经创建的 commit。
- commit 遵循每个仓库的身份、hook、`core.hooksPath` 和签名配置。需要交互时使用文本模式
  `--jobs 1`；机器模式不会把 hook 或 Git 原始输出写进 receipt。批量部分成功不自动 reset、
  amend、rebase 或回滚其他仓库的提交。
- plan 中 add/unstage 的风险为 `index`、副作用为 `git_indexes`；commit 风险为
  `local_history`，副作用为 `git_objects`、`local_refs`、`git_indexes`、`hooks`。apply 只核对
  workspace revision，不冻结 HEAD、index 或工作树。

推荐使用两个独立 plan/apply，并在中间审阅 index。每次 apply 都复制紧邻 plan 返回的
`workspace.revision`；commit apply 必须重复 plan 时完全相同的选择器和 message：

```sh
# 1. 计划并执行全量暂存。
batch-git --output json --plan add --match 'service-*'
batch-git --output json --apply --expect-workspace-revision 'sha256:<add-plan-revision>' \
  add --match 'service-*'

# 2. 在提交前审阅实际 index。
batch-git exec --match 'service-*' -- diff --cached --stat

# 3. 用同一条消息计划并执行 commit；复制 commit plan 自己的 revision。
batch-git --output json --plan commit --match 'service-*' -m 'Update generated clients'
batch-git --output json --apply --expect-workspace-revision 'sha256:<commit-plan-revision>' \
  commit --match 'service-*' -m 'Update generated clients'
```

## 远端、分支与定时任务

- 实际 `push` 前先运行 `push --dry-run`；只有明确授权时使用 `--set-upstream`，绝不 force push。
- `checkout` 前检查工作树与分支；远端同名分支歧义必须传 `--remote`。`merge` 需要明确来源模式
  （显式分支、`--feature` 或 `--default`）、范围和授权；`--update-current` / `--uc` 只允许以
  ff-only 更新当前目标分支，`--refresh-source` / `--rs` 会 fetch 后将最新 remote-tracking 来源
  合入当前分支，不移动本地来源分支。当前目标没有 upstream 的仅本地分支使用 `--uc` 时会跳过 pull，
  并继续合并来源。`--default` 按仓库读取清单默认分支；未使用 `--rs` 时本地
  不存在才回退到各自 `primary_remote`。`BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE`（默认 true）只影响
  `merge --default` 的来源刷新，`BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT`（默认 false）只影响
  `merge --feature` 的当前分支更新；CLI 的启用或 `--no-*` 关闭选项优先。不要自动解决、abort 或继续冲突。
- schedule 使用其专用流程：`schedule plan` → `doctor` → `generate` 或 `register --dry-run` →
  明确授权后的 `register`。不要使用全局 `--plan schedule …`，该组合会被拒绝以避免语义混淆。
  `register`、`unregister`、`remove --unregister` 会改变原生调度器，必须单独得到授权。
- 已注册 schedule 的隐藏 `native-run` child 始终携带 `--non-interactive`；在注册前配置可用的
  非交互凭据，不能依赖 Git 终端提示。

## 报告

结束时报告工作区、选择条件、精确命令、receipt 的请求 ID（如有）、每个仓库的状态和 reason
code、最终退出码，以及需要人工处理的事项。不要在报告、命令或结构化数据中复制凭据、私有
remote URL user-info 或 Git 子进程原始输出。
