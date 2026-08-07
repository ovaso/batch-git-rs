# 变更记录

[English](CHANGELOG.md)

本文遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，并采用
[语义化版本](https://semver.org/lang/zh-CN/)。当前项目版本见 `Cargo.toml`。

## [Unreleased]

## [0.4.5] - 2026-08-07

### Fixed

- 修正包元数据、安装器下载和 release/attestation 链接，使其指向 `ovaso/batch-git-rs`，同时为兼容性保留旧的 v1 自动化 schema 标识；release badge 改为按语义版本选择最新 tag，安装文档不再在 crate 发布到 crates.io 之前宣称可直接安装。

## [0.4.4] - 2026-08-07

### Added

- release 归档加入 `LICENSE`、英文/简体中文 README 和 Bash/Zsh/Fish/PowerShell 补全；新增显式版本、无 sudo、
  SHA-256 校验的 macOS/Linux 与 Windows 用户目录安装器，并由 release workflow 为最终归档签发
  GitHub build provenance attestation；同时提供 cargo-binstall release 元数据。
- 公开项目文档改为英文主版本，并提供完整简体中文镜像、双向语言导航和双语 GitHub 贡献模板。

### Changed

- GitHub Actions 改用受支持的 macOS 15 runner 标签；同一分支或 PR 的过期 CI 会被取消；release 会在启动平台构建前验证远端 tag，并串行处理同一 tag 的发布运行。
- 将文档声明和 CI 验证的最低 Rust 版本从 1.85 提升到 1.88，与当前源码使用的语言特性保持一致。
- 将命令编排按 plan、自动化发现、工作区生命周期、只读查询、同步/远端、暂存提交、分支与 Git
  透传拆分为独立模块；工作区生命周期进一步分离 clone、scan、restore 和清单 membership，只读
  查询分离 list、status、find、info 和 branch。schedule 命令再按声明、执行、查询、本机生命周期
  与纯辅助规则拆分，并继续隔离三平台原生产物、注册系统调用和注册状态；CLI 路径、退出码、
  JSON/JSONL 字段、capabilities command 字符串和调度行为保持不变。
- 将 Git 执行、克隆、检出、检查、远端、发现与结果类型，以及 CLI invocation/command/领域参数、
  report 结果/JSONL/机器输出/文本输出、automation 选项/协议输出/错误分类边界拆分为 facade 后的
  独立模块；新增默认启用的 `schedule` Cargo feature，普通构建保持完整命令面，精简构建可条件
  编译移除调度命令和原生集成。
- 以 `cli::metadata` 编译期表集中规范命令名、兼容别名、可写性、全局 plan 支持和 capabilities
  暴露；dispatch 与 plan 继续保持穷尽 match，未改变 CLI 路径、command 字符串或公开语义。
- Git 子进程 stdout/stderr 改为持续排空但各自最多保留 1 MiB 的头尾窗口，超限时插入稳定截断
  标记，避免并发大输出造成无界内存增长。

### Fixed

- 清单仓库路径除词法校验外，还会校验现有路径或最近存在祖先的 canonical 路径；允许仍位于
  工作区内的 symlink，拒绝通过 symlink 把现有仓库或 clone/restore 目标解析到工作区外。
- 顶层机器错误码改由类型化 `ErrorCode` 决定，不再对自然语言消息执行 `lock`、`schedule`、
  `timed out` 等字符串匹配，避免上下文文案导致误分类。

### Validation

- CI 与 Release 平台构建会记录 Linux/macOS 动态依赖检查结果；贡献指南补充了受保护主分支的 PR 流程与合并后打 tag 的发布流程。
- 增加顶层扁平命令面与完整 capabilities 命令名册回归，防止后续内部整理意外改名或引入嵌套路径。
- 增加无默认 features 的编译、CLI 命令面和 capabilities 名册验证；默认 features 继续执行完整
  schedule、automation protocol 与 MVP 回归。
- CI 新增无默认 features 的 check、完整测试、Clippy、release build，以及精简帮助、capabilities
  和缺失 schedule 命令的行为验证。
- 增加工作区外 symlink 黑盒回归、类型化错误分类单元测试和多 MiB 子进程输出有界保留测试。

## [0.4.3] - 2026-08-06

### Added

- 新增只读命令 `batch-git env list`（别名 `env ls`），集中展示支持的环境变量、默认值和本次调用的
  最终生效值；使用 `-d` / `--description` 时增加说明，文本模式将 `CURRENT` 列在兼容终端中显示为
  绿色，并支持 v1 JSON / JSONL 与 capabilities 发现。

### Changed

- 合并默认分支时，`BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE` 默认开启，自动刷新并合并远端最新
  default；通过 `merge --feature` 合并特性分支时，`BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT` 默认
  关闭，仅在配置开启时更新当前分支。`--uc`、`--rs` 与各自 `--no-*` 选项始终优先。
- 移除不区分来源类型的 `BATCH_GIT_MERGE_UPDATE_CURRENT` 与
  `BATCH_GIT_MERGE_REFRESH_SOURCE` 环境变量。

### Fixed

- 文本表格在交互终端中按可用列宽对超长单元格软换行，续行保持原列起点对齐，不再由终端硬换行
  后落到第一列；ANSI 颜色和 Unicode 宽度计算在换行后仍保持正确。

## [0.4.2] - 2026-08-06

### Added

- `merge` 新增 `--refresh-source`（`--rs`）：刷新来源远端引用后，合并最新的 remote-tracking
  来源分支而不移动本地来源分支；可与既有 `--update-current` 的新别名 `--uc` 组合，用于先
  ff-only 更新当前目标分支再合并来源。两个行为可由
  `BATCH_GIT_MERGE_UPDATE_CURRENT` / `BATCH_GIT_MERGE_REFRESH_SOURCE` 配置默认值，并由 CLI
  启用或 `--no-*` 关闭选项覆盖。

### Fixed

- `merge --update-current`（`--uc`）在当前目标分支没有 upstream 时跳过 ff-only pull，允许仅本地的
  特性分支继续合并来源分支。

## [0.4.1] - 2026-08-06

### Fixed

- `merge` 在未要求 `--update-current` 时，会先拒绝本地 tracking ref 已显示为落后或分叉的当前
  分支，避免对过期目标分支启动 Git merge 并遗留阻塞后续 `pull` 的进行中合并状态。

## [0.4.0] - 2026-08-04

### Added

- 新增安全批量 `batch-git add`：复用名称、相对目录、`--match` 与 `--all` 选择器，省略选择器时
  覆盖整个工作区；首版不接受文件 pathspec，固定暂存全部非忽略的新增、修改和删除，并拒绝
  未解决冲突；
- 新增 `batch-git commit -m <message>`：只提交既有 index，不隐式 add，不提供 amend、空提交
  或 hook bypass；拒绝 detached HEAD、未解决冲突和进行中的 merge/rebase/cherry-pick/revert
  等 Git operation；
- 新增全量 `batch-git unstage`：只恢复选中仓库的 index、保留工作树且不移动 HEAD；unborn HEAD
  使用 `git read-tree --empty` 安全清空暂存区；
- add/commit/unstage 接入 v1 JSON / JSONL、plan/apply、capabilities 和稳定 reason code；plan
  公开 `git_indexes`、`git_objects`、`local_refs`、`hooks` 等副作用及命令参数边界。

### Fixed

- `unstage` 使用结构化 HEAD 状态区分真正的 unborn 仓库与名为 `(unborn)` 的合法分支，避免在
  后一种情况下错误清空整个 index。
- `commit` 在 Git operation 同时存在未解决冲突时稳定返回 `unresolved_conflicts`，并继续为
  已解决但尚未完成的 merge/rebase/cherry-pick/revert 返回
  `repository_operation_in_progress`。

### Security

- commit 保留仓库既有 hook、身份和签名策略，不提供自动绕过选项；机器模式继续强制非交互。
  hook、filter、签名超时或跨仓库失败不会触发 reset、amend、rebase 等自动回滚，调用方须检查
  部分成功及可能已经更新的本地 ref。

## [0.3.0] - 2026-08-04

### Added

- 新增 `batch-git merge --default`（短参数 `-d`），逐仓库读取 `workspace.toml` 的
  `default_branch` 并合入各自当前分支；支持多级复杂分支名，本地分支不存在时固定回退到该仓库
  的 `primary_remote`，且不会隐式 fetch；
- merge plan 为每个仓库新增 `source_branch`、`source_mode` 和 `remote_fallback`，便于在
  apply 前审阅显式分支、环境特性分支或逐仓库默认分支来源。

### Fixed

- 普通 `merge` 现在正确读取已文档化的 `BATCH_GIT_REMOTE`，可在多个远端存在同名源分支时按
  环境配置消歧。

### Security

- 升级 `git2` 至 `0.21.0`，修复 RUSTSEC-2026-0183 与 RUSTSEC-2026-0184 报告的潜在未定义
  行为；网络操作仍统一由系统 Git 执行，`git2` 保持禁用默认网络特性。

### Validation

- 新增 CLI 与 automation protocol 黑盒回归，覆盖不同复杂默认分支、主远端回退、多远端消歧、
  参数冲突和无副作用 plan；`cargo-deny` 的 advisories、bans、licenses、sources 检查通过。

## [0.2.0] - 2026-07-31

### Added

- GitHub Actions 质量门禁、三平台发布构建、依赖升级与安全审计；
- 贡献、安全、兼容性、架构、自动化契约和 agent 协作说明；
- `batch-git-automation` 仓库内 skill，用于安全编排多仓库操作。
- automation protocol v1：全局 `--output json|jsonl`、`--request-id`、所有公开内建命令与
  schedule 子命令的机器 receipt，以及不污染 stdout、按清单顺序输出的批量仓库事件流；单仓库
  `clone` 同样输出完整的 `started`、`repository_finished`、`finished` 生命周期；
- `capabilities` 与 `schema operation-result|workspace`，使 agent 可从当前二进制发现协议、
  安全边界和 JSON Schema，而不是猜测安装版本；
- `--plan` / `--apply --expect-workspace-revision`：零副作用预览已解析范围、风险和副作用，
  并在执行前核对 `workspace.toml` SHA-256 revision；
- `--non-interactive` 与每个系统 Git 子进程的 `--timeout`；timeout 只终止直接启动的子进程，Git
  后代进程可能继续存活；clone/restore 超时或失败时保留目标目录供人工检查，避免递归删除
  并发写入的内容；
- 原生 schedule 的隐藏 `native-run` child 始终非交互，避免 scheduler 或无终端环境等待 Git
  认证提示，并把 child 在获取锁后发现的 stale revision 保持为
  `stale_workspace_revision`，而非泛化为调度器错误；
- 自动化协议黑盒测试，覆盖新旧 JSON 兼容、结构化参数错误、schedule receipt 和 stale-plan
  拒绝。

### Changed

- `status` 与 `branch` 增加兼容的直接 `--json` payload；已有 `list`、`find`、`info` 和
  schedule `--json` 顶层结构保持不变。
- 机器模式不再转发 Git 或原生调度器的原始 stdout/stderr；批量结果通过稳定 status 和
  `reason_code` 表达，避免凭据或非结构化诊断进入协议。

## [0.1.1] - 2026-07-29

补丁版本，修复 Git 代理、远端配置和定时任务并发问题，并缩小发布产物。

### 修复

- `clone` 改由系统 Git 执行，继续支持指定分支、浅克隆、单分支、自定义远端名、
  Git 凭据配置和交互认证；本地路径浅克隆仍保持真正的 shallow 语义；
- 修复清单未声明独立 `push_url` 时，本地仓库遗留的 push URL 不会被清除、后续
  push 可能发往非清单地址的问题；
- 修复 `--jobs 1` 下 Git 透传的 stdout/stderr 仍被管道捕获，导致 `rebase -i`、
  编辑器和其他 TTY 交互命令失败的问题；
- schedule register/unregister 现在纳入工作区锁，避免与 update/remove 或并发注册
  之间产生旧配置覆盖和注册状态竞态。

### 构建与发布

- 禁用 `git2/libgit2` 的网络特性，网络访问统一交给系统 Git；
- 移除 libssh2 和 OpenSSL 运行时依赖，macOS 产物不再依赖 Homebrew OpenSSL 路径；
- 移除 Chrono 未使用的 Serde 特性；
- release 启用 size 优化、LTO、单 codegen unit、abort panic 和符号剥离；macOS
  arm64 参考产物由约 `4.5 MiB` 降至约 `1.9 MiB`。

### 验证

- 新增系统 Git clone 参数代理和 push URL 同步回归测试；
- 全部单元与端到端测试、Clippy、格式检查和 release 构建通过。

## [0.1.0] - 2026-07-29

首个封版版本。

### 工作区管理

- 支持扫描已有仓库、克隆并登记、从清单恢复、fetch/prune 和安全 sync；
- 使用 `workspace.toml` 保存仓库、远端、默认分支和 schedule 声明；
- 清单校验、原子写入、URL 凭据清理和工作区锁；
- 支持工作区/仓库信息、状态、当前分支、列表和 JSON 输出。

### 分支与 Git 操作

- 支持本地/远端分支搜索、默认分支和特性分支切换、从显式起点创建分支，并可直接合并当前特性分支；
- 支持远端同名分支消歧；
- 支持 merge 前可选 fast-forward-only 更新，以及独立的安全 pull；
- 支持安全批量 push、首次推送时显式建立 upstream、全工作区 Git 透传和按仓库/通配模式选择的 `exec`；
- 对 `git commit` 的 “nothing to commit” 结果按 skipped 聚合。

### 执行体验

- 有界并发、清单顺序输出和仓库级聚合结果；
- 交互终端进度、非交互稳定表格、Unicode 宽度对齐和 `NO_COLOR`；
- 退出码区分完成、仓库级失败和配置/校验错误。

### 定时任务

- 支持 `sync` / `pull`，每日、固定间隔和六段式 cron；
- 支持计划、验证、生成、注册、更新、状态、立即运行、反注册和删除；
- 支持 macOS launchd、Linux systemd user timer 与 Windows Task Scheduler；
- 支持 overlap 策略、注册定义一致性检查和可选后台日志。

### 已知限制

- 清单重写不保留 TOML 注释和原始排版；
- schedule 时区通过 `BATCH_GIT_TZ` 配置，未设置时使用系统时区；
- launchd cron 秒字段必须为 `0`；
- Windows Task Scheduler 暂不支持 cron，固定间隔限制为 `1m` 至 `31d`；
- 不支持 Quartz `L`、`W`、`#` 或年份字段；
- 批量操作不提供跨仓库事务回滚，失败时可能部分成功；
- 交互式 Git 子进程需使用 `--jobs 1`。

[Unreleased]: https://github.com/ovaso/batch-git-rs/compare/v0.4.5...HEAD
[0.4.5]: https://github.com/ovaso/batch-git-rs/compare/v0.4.4...v0.4.5
[0.4.4]: https://github.com/ovaso/batch-git-rs/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/ovaso/batch-git-rs/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/ovaso/batch-git-rs/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/ovaso/batch-git-rs/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/ovaso/batch-git-rs/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/ovaso/batch-git-rs/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/ovaso/batch-git-rs/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/ovaso/batch-git-rs/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/ovaso/batch-git-rs/releases/tag/v0.1.0
