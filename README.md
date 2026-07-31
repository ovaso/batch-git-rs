# batch-git

[![CI](https://github.com/livenv/batch-git/actions/workflows/ci.yml/badge.svg)](https://github.com/livenv/batch-git/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/livenv/batch-git)](https://github.com/livenv/batch-git/releases)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`batch-git` 是一个多仓库 Git 工作区管理工具。它用一份可复制、可审阅的
`workspace.toml` 管理多个相互独立的 Git 仓库，工作区本身不需要是 Git 仓库。

当前封版版本：`0.2.0`。

## 主要能力

- 扫描已有目录并生成工作区清单；
- 从清单恢复缺失仓库，或克隆并自动登记单个仓库；
- 批量 fetch、fast-forward pull、push、checkout、merge 和查看状态；
- 按仓库精确选择或按名称通配后执行原生 Git 命令；
- 在 macOS `launchd`、Linux `systemd --user` 或 Windows Task Scheduler 中注册定时同步；
- 有界并发、稳定输出顺序、工作区锁和聚合退出码；
- 面向 CI 与 agent 的版本化 JSON / JSON Lines 协议、结构化错误、能力发现和 plan/apply
  清单前置条件。

`batch-git` 不会在未明确请求时自动 merge、rebase、stash、reset 或清理工作树。
clone 或 restore 失败/超时时也不会递归删除目标目录；保留的内容须由用户检查并明确处理后才能重试。

## 安装

需要 Rust 工具链和系统 Git。任选一种方式构建：

```sh
# 仅构建
cargo build --release
./target/release/batch-git --version

# 构建并安装到指定目录
BATCH_GIT_INSTALL_PATH="$HOME/.local/bin" ./build.sh
```

确保安装目录已经加入 `PATH`：

```sh
export PATH="$HOME/.local/bin:$PATH"
```

## 快速开始

从现有多仓库目录建立工作区：

```sh
cd /path/to/workspace
batch-git scan
batch-git status
batch-git branch
batch-git fetch
```

用清单恢复工作区：

```sh
mkdir restored-workspace
cp workspace.toml restored-workspace/
cd restored-workspace
batch-git restore
batch-git fetch
```

批量执行原生 Git：

```sh
# 所有已物化仓库
batch-git -- status --short

# 指定仓库
batch-git exec service-api service-web -- log -1 --oneline

# 按仓库名匹配
batch-git exec --match 'service-*' -- fetch --prune
```

命令边界是明确的：

```text
batch-git <command> [options]       # batch-git 内建命令
batch-git -- <git-args...>          # 在全部仓库中原样执行 Git
```

未知内建命令会报错，不会自动解释为 Git 命令。例如 `batch-git branch` 显示
工作区分支摘要，`batch-git -- branch` 才会在每个仓库执行 `git branch`。

## 常用命令

| 场景 | 命令 |
|---|---|
| 创建或补充清单 | `batch-git scan` |
| 查看工作区状态 | `batch-git status` |
| 查看当前分支 | `batch-git branch` |
| 安全更新远端引用 | `batch-git fetch` 或 `batch-git sync` |
| fast-forward 更新当前分支 | `batch-git pull` |
| 推送当前 tracking 分支 | `batch-git push` |
| 切换同名分支 | `batch-git checkout <branch>` |
| 切换各仓库默认分支 | `batch-git cd` |
| 搜索本地或远端分支 | `batch-git find 'feature/*'` |
| 查看工作区或仓库详情 | `batch-git info [repository]` |
| 管理定时任务 | `batch-git schedule --help` |

完整参数以 `batch-git --help` 和各子命令的 `--help` 为准。

## 自动化与 agent

新自动化优先使用全局 `--output json`，而不是解析表格或旧的子命令 `--json`：

```sh
# 先发现当前二进制的能力，再读取稳定 receipt。
batch-git --output json capabilities
batch-git --output json --request-id ci-184 status

# 预览批量同步；apply 会重新核对 plan 返回的 workspace revision。
batch-git --output json --plan sync --match 'service-*'
batch-git --output json --apply --expect-workspace-revision 'sha256:…' \
  sync --match 'service-*'
```

长任务可使用 `--output jsonl` 逐行消费 `started`、`repository_finished` 和 `finished`
事件；单仓库 `clone` 也会按此顺序发出一个仓库终态事件。仓库事件保持清单顺序，已先完成的
后序仓库可能等待前序事件。`--non-interactive` 禁止 Git 提示，`--timeout 5m` 只限制直接启动的
Git 子进程，不能保证终止其认证或传输后代进程。
`schema workspace` 输出的 schema 对应 `workspace.toml` 的 JSON 表示，清单中可省略有默认值的字段。完整字段、
兼容策略和安全边界见[自动化契约](docs/AUTOMATION_CONTRACTS.md)；仓库内 skill 提供 agent
的默认操作流程。

## 文档

- [用户手册](docs/USER_GUIDE.md)：安装、工作流、命令说明和故障排查；
- [工作区清单](docs/WORKSPACE.md)：`workspace.toml` 格式、环境变量和状态目录；
- [定时任务](docs/SCHEDULES.md)：声明、验证、注册、日志和平台差异；
- [开发与发布](docs/DEVELOPMENT.md)：验证命令、实现边界和封版检查；
- [架构说明](docs/ARCHITECTURE.md)：模块职责、并发、锁与 Git 执行边界；
- [兼容性](docs/COMPATIBILITY.md)：Rust、Git、平台与调度器支持矩阵；
- [自动化契约](docs/AUTOMATION_CONTRACTS.md)：面向 CI 和 agent 的版本化 JSON、JSONL、
  plan/apply、schema 与退出码约定；
- [变更记录](CHANGELOG.md)：版本级交付内容和已知限制。

根目录中的 `Request.md`、`OPTIMISE.md` 和 `SUGGESTIONS.md` 是设计过程归档，
不作为当前行为说明。正式行为以命令帮助、上述手册和当前代码为准。

## 退出码

- `0`：命令完成；允许预期内的 checkout skip；
- `1`：至少一个仓库操作失败；
- `2`：参数、环境变量、工作区、清单校验或文件写入错误。

## 开发验证

```sh
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --locked --release
```

## 参与项目

提交变更前请阅读 [贡献指南](CONTRIBUTING.md)、[安全政策](SECURITY.md) 和
[agent 协作约定](AGENTS.md)。对 multi-repository Git 操作进行自动化时，可使用仓库内
未安装的 [batch-git automation skill](skills/batch-git-automation/SKILL.md)。

## 许可证

本项目使用 [MIT License](LICENSE)。
