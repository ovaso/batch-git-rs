# 变更记录

本文采用面向发布的变更记录格式。当前项目版本见 `Cargo.toml`。

## 0.1.1 - 2026-07-29

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

## 0.1.0 - 2026-07-29

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
