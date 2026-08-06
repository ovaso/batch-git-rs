# batch-git 自动化能力名册

以当前源码中的 automation protocol v1 为基准；安装的二进制可能较旧。每次自动化先执行
`batch-git --output json capabilities`，再按需执行 `batch-git schema operation-result`。不要
仅根据 skill、README 或版本号猜测可用参数。

## 自动化基础能力

| 能力 | 命令 | agent 使用规则 |
|---|---|---|
| 能力发现 | `capabilities` | 每次新环境的起点；读取协议版本、格式、命令、安全边界。 |
| schema 发现 | `schema operation-result`、`schema workspace` | 校验 receipt 核心字段，同时允许 future optional fields；workspace schema 接受省略的默认清单字段。 |
| 一次性 receipt | `--output json <command>` | 新自动化的默认格式；stdout 只能有一个 v1 JSON 文档。 |
| 长任务事件 | `--output jsonl <command>` | 消费 `started`、`repository_finished`、`finished`；批量操作及单仓库 `clone` 都产生仓库终态事件，事件按清单顺序输出而非完成时间顺序；以最终事件和退出码结论。 |
| 关联追踪 | `--request-id <id>` | 将调用方的 1–128 字符非控制 ID 回显到所有 machine output。 |
| 交互/时间控制 | `--non-interactive`、`--timeout 30s|5m|1h` | 无人值守文本操作禁用提示；timeout 只终止直接 Git child，不限制工作区锁，后代 helper 可能存活。 |
| 预览与前置条件 | `--plan` → `--apply --expect-workspace-revision <digest>` | 先读取 plan 的范围、风险与 revision；清单改变后重新 plan。apply 不保留远端、HEAD、index 或工作树状态。 |

旧的子命令 `--json` 仍可兼容历史脚本，但不是统一 schema。新调用使用全局 `--output json`；
二者同时出现时全局输出格式优先。

| 能力 | 命令 | 自动化价值 | 安全等级 | agent 使用规则 |
|---|---|---|---|---|
| 工作区发现与校验 | `info`、`list` | 确定清单根目录、仓库名和机器可读输入 | 只读 | 所有流程的起点；优先 `--output json`。 |
| 运行配置发现 | `env list`（别名 `env ls`） | 列出支持的环境变量、默认值和最终生效值 | 只读 | 无需工作区；自动化读取 `data.variables[]`，需要说明时加 `-d`。不要解析绿色文本列；输出可能包含本机绝对路径。 |
| 运行态盘点 | `status`、`branch`、`find` | 识别 dirty、ahead/behind、当前分支和目标分支覆盖面 | 只读 | 任何本地或远端写操作前后执行。`find --remote` 仅查询已 fetch 的远端引用。 |
| 全量暂存 | `add [selectors/--match/--all]` | 将选中仓库全部非忽略新增、修改和删除写入 index | 本地 index 写入 | 省略选择器时全工作区；首版无文件 pathspec，拒绝未解决冲突。add 后必须审阅 staged diff。 |
| 本地提交 | `commit [selection] -m <message>` | 用同一消息提交各仓库已有 index | 本地历史写入、高风险 | 不 implicit add/amend/empty/hook bypass；拒绝 detached、冲突和进行中的 Git operation；hook/签名可导致部分失败。 |
| 全量撤销暂存 | `unstage [selectors/--match/--all]` | 恢复选中 index，同时保留工作树 | 本地 index 写入 | 首版无文件 pathspec；不移动 HEAD，unborn 使用 read-tree empty；不能撤销 commit。 |
| 由现有仓库建清单 | `scan [--depth N]` | 将散落多仓库转为可复现工作区 | 清单写入 | 只增补、不删除登记；先 `--plan`，再确认根目录。 |
| 受控加入仓库 | `clone <url> [directory]` | 单仓库克隆成功后自动登记 | 本地写入 + 网络 | 目标必须是工作区内不存在的相对目录；失败/超时时保留目标供人工检查，不自动递归删除；处理该目录后才能重试。JSONL 也产生完整生命周期，失败事件以请求目录标识仓库。不要在输出中暴露 URL 凭据。 |
| 缺失仓库恢复 | `restore` | 从清单重建缺失 checkout | 本地写入 + 网络 | 不覆盖已存在目录；失败/超时目标会保留供人工检查；必须从含 `workspace.toml` 的工作区根目录调用。 |
| 元数据维护 | `forget <selector>` | 停止管理一个仓库而保留目录 | 清单写入 | 先 `--plan` 展示精确匹配，再要求确认。 |
| 无工作树更新 | `fetch` | 更新所有远端引用并 prune | 网络、低风险 | 不 merge、不 checkout；适合自动化前的刷新。 |
| 无人值守同步 | `sync [selectors/--match/--all]` | `restore + fetch`，保证缺失仓库恢复并更新 refs | 本地写入 + 网络、低风险 | 首选后台操作；不改已有工作树。 |
| 快进更新 | `pull [selectors/--match]` | 将当前 tracking 分支 fast-forward 到远端 | 工作树写入 + 网络 | 仅在范围和干净状态已确认时使用；不绕过失败。 |
| 安全发布 | `push [selection] [--dry-run] [-u/--set-upstream]` | 批量发布当前分支 | 远端写入 | 先 `--dry-run`，实际推送需用户授权；无 force push。 |
| 目标分支切换/创建 | `checkout`、`cd`、`cf` | 跨仓库切换同名分支、默认分支或特性分支 | 工作树写入 | 先查状态和分支。远端歧义必须明确 `--remote`。 |
| 跨仓库合并 | `merge <branch>`、`merge --feature`、`merge --default` | 将同一源分支或各仓库声明的默认分支合入当前分支 | 工作树写入、高风险 | 必须有明确来源、范围和授权；`--uc` 先 ff-only 更新当前目标，`--rs` fetch 后合并最新 remote-tracking 来源；冲突留给人工处理。 |
| 原生 Git 逃生舱 | `-- <git-args>`、`exec ... -- <git-args>` | 覆盖没有内建命令的 Git 只读或精细动作 | 随传入命令变化 | 保留 `--`；默认只允许低风险只读命令。 |
| 定时声明 | `schedule add/update/list/plan/doctor/generate` | 用可审阅声明规划跨平台自动同步 | 清单写入或只读 | 优先 `sync` action；使用 schedule 自己的 plan、doctor、generate，不能用全局 `--plan schedule …`。 |
| 定时器生命周期 | `schedule register/unregister/remove/run/status` | 注册、检查、立即执行或清理原生调度任务 | 外部系统写入 | `register/unregister/remove --unregister` 需要明确授权；先 dry-run/generate。已注册任务的 hidden native-run child 强制非交互。 |

## 选择器与并发

- `add`、`commit`、`unstage`、`sync`、`pull`、`push` 接受精确名称/相对目录、可重复 `--match`，
  或 `--all`；省略选择器时默认工作区全量。暂存三命令首版不接受文件 pathspec。
- `exec` 需要至少一个精确选择器或 `--match`；全量透传改用 `batch-git -- <git args>`。
- `find --repo` 只根据规范仓库名称过滤。通配符只支持 `*`，并且大小写敏感。
- 并发优先级为 `--jobs`、`BATCH_GIT_JOBS`、默认 `4`。即使输出稳定排序，任何并行写入仍可能造成多个仓库同时改变。

## 关键保障与边界

- 工作区锁 `.workspace.lock` 串行化会执行 Git 或修改清单的进程。
- `pull` 为 fast-forward-only；`batch-git` 不会自动 merge、rebase、stash、reset 或清理工作树。
- `add` 拒绝未解决冲突；`commit` 只提交 index，拒绝 detached HEAD 和进行中的 Git operation；
  `unstage` 只改 index 并保留工作树。capabilities 的 `safety` 会公开
  `commit_stages_content=false`、`add_rejects_unresolved_conflicts=true`、
  `commit_rejects_repository_operations=true` 和 `unstage_preserves_working_trees=true`。
- `push` 只推当前分支；只有 `--set-upstream` 才会创建远端分支和 tracking。
- `checkout` 目标缺失为 skip；远端同名分支多于一个则是待消歧错误。
- 批量任务允许部分成功：以每仓库结果和最终退出码共同判定，不能只看一行摘要。
- 退出码：`0` 完成（允许预期 skip），`1` 至少一个仓库操作失败，`2` 参数、配置、清单或文件错误。
- `--output json` 中，退出码 `1` 仍会提供完整 `data.results[]`；退出码 `2` 读取顶层
  `error.code`，不要解析 message。常见仓库级 code 有 `dirty_worktree`、`no_upstream`、
  `branch_ambiguous`、`branch_missing`、`repository_unavailable`、`nothing_to_stage`、
  `nothing_to_unstage`、`nothing_to_commit`、`unresolved_conflicts`、`detached_head`、
  `repository_operation_in_progress`、`timeout` 和 `git_exit`。
- 所有公开内建命令和 schedule 子命令均支持 v1 JSON/JSONL。`exec` 与顶层 Git 透传的
  结构化结果风险为 `unclassified`，不要据此自动放宽授权标准。
- hook、filter、签名和 helper 可能在直接 Git child timeout 后继续；commit ref 也可能已经更新。
  批量部分成功不会自动 reset、amend、rebase 或跨仓库回滚。
