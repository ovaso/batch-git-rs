# 架构说明

[English](../ARCHITECTURE.md)

## 目标与边界

`batch-git` 协调多个独立 Git 工作树，而不是创建新的 monorepo。`batchspace.toml` 保存可复制
的恢复声明；当前分支、HEAD、工作树变更和远端引用始终从本地 Git 仓库实时读取。

```text
CLI / env ──> cli + settings + automation ──> commands / schedule
                                  │
                     workspace ──┼── model (batchspace.toml validation)
                     lock/write  │
                                  ├── git2: inspection, staging facts and local ref operations
                                  └── system Git: add/commit/unstage, merge/pull/push,
                                                  clone/fetch and passthrough
                                           │
                                      report + table + color
```

## 模块职责

- `cli` 通过 facade 保持 `crate::cli::*` 稳定；`invocation` 在显式 `--` 处分离原生 Git 透传，
  `command` 定义 clap 顶层命令面，`args` 按自动化、工作区、同步、分支、查询、执行和 schedule
  领域保存参数模型。
- `automation` facade 保持协议调用路径稳定；`options` 负责输出格式、request ID、timeout 与
  plan/apply 选项校验，`output` 只负责 v1 JSON envelope、JSONL 生命周期和 workspace revision
  上下文序列化，`error` 集中稳定错误分类与 URL user-info 脱敏。
- `settings` 统一处理 CLI、环境变量和默认值的优先级；`env list` 复用同一解析路径展示最终生效值，
  不维护第二份运行配置。
- `commands` 只在 `mod.rs` 保留顶层分派、plan/apply revision 校验、统一子进程策略与批量进度；
  `plan`、`automation_commands`、`remote`、`changes`、`branches` 和 `exec` 分别承载对应领域工作流。
  `workspace_commands` 再按 clone、scan、restore、manifest membership 与共享路径/命名不变量拆分；
  `inspect` 只作为 facade，list、status、find、info、branch 各自维护查询数据模型和失败语义。
  单仓库失败应转为可聚合结果，不能取消其他仓库。
- `workspace` 负责根目录发现、排他锁和 `batchspace.toml` 原子替换。
- `model` 定义 schema version 1、跨字段校验和可序列化模型。
- `git` facade 保持调用路径稳定；`types` 是值对象，`execution` 统一系统 Git 环境隔离、交互和
  timeout，`clone`、`checkout`、`inspect`、`remotes`、`discovery` 分别承担克隆、分支切换、
  只读事实、远端配置和工作树发现。add/commit/unstage、merge/pull/push 等仍由命令层通过统一
  执行策略调用系统 Git。
- `schedule` 的 `commands` facade 使用一次调用一个轻量 `CommandContext`，统一并发、输出和 revision
  策略；其下 `declarations`、`execution`、`query`、`native`、`support` 分别处理清单声明、计划/运行、
  只读查询、本机任务生命周期和纯查找/显示规则。`artifact` 明确生成 launchd、systemd user timer
  与 Windows Task Scheduler 定义，`registration` 隔离原生系统调用，`state` 维护并校验本地注册
  摘要；任何系统路径都不写入共享清单。
- `parallel` 保证并发上限和输入顺序收集；`report` 由 `result`、`jsonl`、`machine`、`text` 和
  `child_output` 分离业务结果、生命周期、协议序列化、文本摘要和子进程输出块；`table` 负责稳定
  对齐。

默认 Cargo feature `schedule` 编译完整调度命令和原生集成；关闭默认 features 时，CLI 和
`capabilities.commands` 同时移除 schedule，但 `model` 仍解析并保留清单中的 schedule 声明。
launchd、systemd 和 Windows artifact 均保留跨平台生成能力，只有真实宿主系统差异使用
`target_os` 条件编译。

`cli::metadata` 是规范命令名、兼容别名、可写性、全局 plan 支持和 capabilities 暴露的编译期
事实来源；`Command::kind()`、dispatch 和 plan 保持穷尽 match，使新增枚举变体时由编译器强制
补齐处理分支，而不引入动态注册表。

`--output json` 的输出边界在 `automation`：成功调用只能产生一个 receipt，错误也由库入口
转换为结构化 document。`report` 将批量 `RepositoryResult` 映射为稳定的 per-repository
records，且不把子 Git stdout/stderr 放入协议。`--output jsonl` 在相同数据模型之上输出
生命周期和仓库终态事件；`clone` 将其单个受控 Git 操作也建模为一个仓库事件，避免为单仓库
操作提供不同的进度协议。
顶层 `error.code` 只从错误链中的类型化分类读取，`anyhow` 继续承载上下文；自然语言变化或底层
输出中偶然出现 `lock`、`schedule`、`timeout` 等词不会改变机器分类。

## 一致性与副作用

所有可能执行 Git 或写入清单的工作流都使用工作区隐藏的 `.batchspace.lock`。清单写入先写同目录临时
文件，再同步并原子替换。批量命令不是跨仓库事务：每个仓库独立完成或失败，最终报告必须保留
success、skipped 与 failed 的差别。

`sync` 是无人值守默认操作：恢复缺失仓库并 fetch/prune，不修改已存在的工作树。`pull` 固定为
fast-forward-only。`add` 只暂存全部非忽略的新增、修改和删除，并在未解决冲突时拒绝；`commit`
只提交既有 index，拒绝 detached HEAD 和正在进行的 Git operation，不提供 implicit add、amend、
空提交或 hook bypass；`unstage` 全量恢复 index 且不改工作树，unborn HEAD 使用
`git read-tree --empty`。重写历史、清理工作树或强制推送仍不是内建自动化能力。

`merge` 默认只使用现有本地引用。显式 `--update-current` / `--uc` 会先对有 upstream 的当前目标分支执行
fast-forward-only pull；没有 upstream 的仅本地分支会跳过此步骤。显式 `--refresh-source` / `--rs` 会 fetch 声明远端并合并最新的
remote-tracking 来源分支，而不移动本地来源分支。两者都可能修改工作树，冲突一律保留给用户处理，
不自动 abort、continue、rebase 或回滚。
`merge --default` 默认启用来源刷新，可由 `BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE` 关闭；
`merge --feature` 默认不更新当前目标分支，可由 `BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT` 开启。
这两个按来源类型的默认值只在相应模式生效，显式 CLI 选项始终优先。

commit 会遵循仓库配置的 hook、身份和签名程序，它们可能产生 batch-git 无法分类的本地或外部
副作用。系统 Git 的单仓库 index/ref lock 与工作区锁共同降低并发冲突，但外部原生 Git 不遵循
`.batchspace.lock`。批量命令不是事务：某些仓库已创建提交后，后续仓库失败或超时不会触发自动
reset、amend、rebase 或其他回滚。直接 Git child 超时后，hook、filter 或签名后代仍可能存活，
调用方必须重新检查 HEAD、index 和工作树。

计划不是事务：`--plan` 只读取本地状态并返回 `batchspace.toml` digest；`--apply` 在持锁后
重新核对该 digest，然后才执行可写命令。add/commit/unstage plan 还公开固定的 index 范围、
提交消息或工作树保留属性，但不会冻结 HEAD、index 或工作树。它不保存额外账本、不会锁住远端，
也不承诺跨仓库回滚。
系统 Git 由 `GitExecutionOptions` 统一控制 stdin、`GIT_TERMINAL_PROMPT` 和单子进程 timeout；
机器输出强制非交互，以免子进程流污染 JSON。捕获线程始终排空 stdout/stderr，但每个流只保留
1 MiB 的头尾诊断窗口，避免并发 `log`、`diff` 或错误输出导致无界内存增长。

清单目录先经过词法相对路径校验，再由 `workspace::repository_path` 解析真实文件系统边界。
已存在路径和最近存在祖先都必须 canonicalize 到工作区根内；工作区内 symlink 可用，指向外部的
仓库或待创建子目录会在读取清单和每次仓库操作前被拒绝。

## 演进规则

- `batchspace.toml` 的 schema 由 `version` 保护；任何破坏性格式调整必须引入迁移和新的主版本。
- v1 JSON / JSONL 输出和公开 schema 是 automation contract；仅新增可选字段可在同主版本内发布。
- 新的 scheduler 平台应实现生成、验证、注册、状态和安全反注册，并在目标平台 CI 测试。
- 新命令必须定义选择器、并发、退出码、部分失败和文档行为，不能只提供 happy path。
- 命令帮助与元数据的后续整理按 [命令组织计划](COMMAND_ORGANIZATION_PLAN.md) 演进；在该计划
  明确进入实施阶段前，不引入嵌套命令路径，也不改变现有命令字符串。
