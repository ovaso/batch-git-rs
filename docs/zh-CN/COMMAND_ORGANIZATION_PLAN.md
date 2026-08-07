# 命令组织计划

[English](../COMMAND_ORGANIZATION_PLAN.md)

本计划只讨论后续的命令元数据与帮助呈现整理。当前重构不改变 CLI 路径、参数、退出码、机器输出、
capabilities command 字符串或 schedule 行为，也不把现有命令改成 `workspace scan`、`repo pull`
等嵌套形式。

## 目标

- 让新增命令时需要维护的事实集中，减少在 `Command`、可写性判断、dispatch、plan 和
  capabilities 之间遗漏同步的风险；
- 改善顶层帮助的浏览效率，同时保留脚本、automation contract 和 skill 已使用的扁平命令路径；
- 继续明确 `fetch` / `sync` / `pull`、`list` / `status` / `info` 的安全语义差异，不通过合并命令
  模糊副作用边界。

## 非目标

- 不重命名、删除或合并现有命令；
- 不改变 `cd`、`cf`、`cc` 等兼容入口的解析结果；
- 不改变 plan/apply、选择器、并发、退出码、JSON/JSONL、reason code、capabilities 或 schema；
- 不为命令分类引入第二份持久化状态，`batchspace.toml` 仍是唯一工作区事实来源。

## 实施状态与后续阶段

### 1. 冻结公开命令面

以黑盒测试锁定顶层命令路径和 capabilities 中的稳定 command 字符串。任何后续实现都必须先证明
现有调用仍可解析，并运行 `tests/automation_protocol.rs` 与主要端到端工作流。

### 2. 单一命令元数据来源（已实施内部边界）

`cli::metadata` 已以编译期表集中并派生或校验：

- 规范 command 字符串和兼容别名；
- 是否可能产生副作用，以及全局 `--plan` / `--apply` 是否适用；
- 顶层帮助分类；
- capabilities 是否公开；
- dispatch 与 plan 是否已有处理分支。

该模型只保存编译期元数据，不保存运行状态。clap 参数类型、dispatch 和 plan 仍使用穷尽 match，
由编译器保证新增枚举变体必须补齐；测试校验 clap 顶层名册与元数据完全一致。没有引入动态注册表。

### 3. 增加非破坏性帮助索引

在不移动命令路径的前提下，为顶层帮助增加快速索引：

- 建立工作区：`scan`、`clone`、`restore`、`forget`；
- 查看：`list`、`status`、`info`、`branch`、`find`、`env`；
- 同步与远端：`fetch`、`sync`、`pull`、`push`；
- 分支：`checkout`、`merge`；
- 暂存与提交：`add`、`unstage`、`commit`；
- 自动化：`capabilities`、`schema`、`schedule`；
- 逃生舱：`exec` 与显式 `-- <git-args...>`。

帮助索引只是导航，不改变各命令的 clap 定义、参数或排序。实施时需要同步帮助快照、中英文
用户手册和两份 CHANGELOG，但不修改 automation command 字符串。

### 4. 降低快捷入口噪音

单独评估将 `cd`、`cf` 从默认命令列表隐藏但继续支持直接调用。它们仍分别等价于
`checkout --default` 与 `checkout --feature`，不移除、不改名，也不改变 preflight command
归一化。该阶段必须增加 `cd`、`cf` 可调用的黑盒回归，并确认 shell completion 与错误文案影响。

## 实施门禁

任何命令呈现变更都应同时检查：

1. 顶层和子命令 `--help`；
2. `Command::is_mutating`、dispatch、plan 和 capabilities 覆盖；
3. 中英文用户手册、automation contracts、CHANGELOG 与仓库内 automation skill 是否需要同步；
4. CLI 单元测试、`tests/automation_protocol.rs` 和 `tests/mvp.rs`；
5. Linux、macOS、Windows 构建，以及 schedule 平台差异是否仍明确。
