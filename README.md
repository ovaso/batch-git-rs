# batch-git

`batch-git` 是一个多仓库 Git 工作区管理工具。它用一份可复制、可审阅的
`workspace.toml` 管理多个相互独立的 Git 仓库，工作区本身不需要是 Git 仓库。

当前封版版本：`0.1.0`。

## 主要能力

- 扫描已有目录并生成工作区清单；
- 从清单恢复缺失仓库，或克隆并自动登记单个仓库；
- 批量 fetch、fast-forward pull、checkout、merge 和查看状态；
- 按仓库精确选择或按名称通配后执行原生 Git 命令；
- 在 macOS `launchd` 或 Linux `systemd --user` 中注册定时同步；
- 有界并发、稳定输出顺序、工作区锁和聚合退出码。

`batch-git` 不会在未明确请求时自动 merge、rebase、stash、reset 或清理工作树。

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
| 切换同名分支 | `batch-git checkout <branch>` |
| 切换各仓库默认分支 | `batch-git cd` |
| 搜索本地或远端分支 | `batch-git find 'feature/*'` |
| 查看工作区或仓库详情 | `batch-git info [repository]` |
| 管理定时任务 | `batch-git schedule --help` |

完整参数以 `batch-git --help` 和各子命令的 `--help` 为准。

## 文档

- [用户手册](docs/USER_GUIDE.md)：安装、工作流、命令说明和故障排查；
- [工作区清单](docs/WORKSPACE.md)：`workspace.toml` 格式、环境变量和状态目录；
- [定时任务](docs/SCHEDULES.md)：声明、验证、注册、日志和平台差异；
- [开发与发布](docs/DEVELOPMENT.md)：验证命令、实现边界和封版检查；
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
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```
