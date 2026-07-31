# 架构说明

## 目标与边界

`batch-git` 协调多个独立 Git 工作树，而不是创建新的 monorepo。`workspace.toml` 保存可复制
的恢复声明；当前分支、HEAD、工作树变更和远端引用始终从本地 Git 仓库实时读取。

```text
CLI / env ──> cli + settings + automation ──> commands / schedule
                                  │
                     workspace ──┼── model (workspace.toml validation)
                     lock/write  │
                                  ├── git2: inspection and local ref operations
                                  └── system Git: network, merge/pull/push, passthrough
                                           │
                                      report + table + color
```

## 模块职责

- `cli` 解析内建命令，并在显式 `--` 处分离原生 Git 透传。
- `automation` 定义版本化 JSON/JSONL envelope、结构化顶层错误、request ID、plan/apply
  revision 前置条件和子 Git 进程执行约束。
- `settings` 统一处理 CLI、环境变量和默认值的优先级。
- `commands` 编排工作区命令；单仓库失败应转为可聚合结果，不能取消其他仓库。
- `workspace` 负责根目录发现、排他锁和 `workspace.toml` 原子替换。
- `model` 定义 schema version 1、跨字段校验和可序列化模型。
- `git` 用 `git2` 读取本地状态，用系统 Git 执行网络和兼容性敏感操作。
- `schedule` 将声明翻译为 launchd、systemd user timer 或 Windows Task Scheduler，并维护
  本地注册摘要；不把系统路径写入共享清单。
- `parallel` 保证并发上限和输入顺序收集；`report`/`table` 保证稳定、可读的结果输出。

`--output json` 的输出边界在 `automation`：成功调用只能产生一个 receipt，错误也由库入口
转换为结构化 document。`report` 将批量 `RepositoryResult` 映射为稳定的 per-repository
records，且不把子 Git stdout/stderr 放入协议。`--output jsonl` 在相同数据模型之上输出
生命周期和仓库终态事件；`clone` 将其单个受控 Git 操作也建模为一个仓库事件，避免为单仓库
操作提供不同的进度协议。

## 一致性与副作用

所有可能执行 Git 或写入清单的工作流都使用工作区 `.workspace.lock`。清单写入先写同目录临时
文件，再同步并原子替换。批量命令不是跨仓库事务：每个仓库独立完成或失败，最终报告必须保留
success、skipped 与 failed 的差别。

`sync` 是无人值守默认操作：恢复缺失仓库并 fetch/prune，不修改已存在的工作树。`pull` 固定为
fast-forward-only。需要修改历史、清理工作树或强制推送的 Git 命令不是内建自动化能力。

计划不是事务：`--plan` 只读取本地状态并返回 `workspace.toml` digest；`--apply` 在持锁后
重新核对该 digest，然后才执行可写命令。它不保存额外账本、不会锁住远端，也不承诺跨仓库回滚。
系统 Git 由 `GitExecutionOptions` 统一控制 stdin、`GIT_TERMINAL_PROMPT` 和单子进程 timeout；
机器输出强制非交互，以免子进程流污染 JSON。

## 演进规则

- `workspace.toml` 的 schema 由 `version` 保护；任何破坏性格式调整必须引入迁移和新的主版本。
- v1 JSON / JSONL 输出和公开 schema 是 automation contract；仅新增可选字段可在同主版本内发布。
- 新的 scheduler 平台应实现生成、验证、注册、状态和安全反注册，并在目标平台 CI 测试。
- 新命令必须定义选择器、并发、退出码、部分失败和文档行为，不能只提供 happy path。
