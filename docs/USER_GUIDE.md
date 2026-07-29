# batch-git 用户手册

本文面向日常使用者。清单字段见 [WORKSPACE.md](WORKSPACE.md)，定时任务的完整
操作说明见 [SCHEDULES.md](SCHEDULES.md)。

## 1. 基本概念

一个 batch-git 工作区由以下内容组成：

```text
workspace/
├── workspace.toml
├── service-api/
├── service-web/
└── service-worker/
```

每个子目录是独立 Git 仓库；`workspace.toml` 只保存恢复仓库所需的稳定信息，
不会缓存当前分支、HEAD、分支列表或 dirty 状态。

除 `scan`、`restore` 外，普通命令会从当前目录向上查找最近的
`workspace.toml`。也可用绝对路径环境变量 `BATCH_GIT_WORKSPACE` 明确指定工作区。
`scan` 和 `restore` 始终以当前目录为工作区根目录。

## 2. 安装与验证

前置条件：

- 可用的 Rust 工具链；
- 系统 Git；
- 使用定时任务时，macOS 需要 `launchd`，Linux 需要 `systemd --user`，Windows
  使用 Task Scheduler。Windows 首版不支持 cron 声明。
- schedule 时区通过 `BATCH_GIT_TZ` 设置；未设置时使用系统时区。

```sh
cargo build --release
./target/release/batch-git --help
```

安装脚本要求明确指定目标目录：

```sh
BATCH_GIT_INSTALL_PATH="$HOME/.local/bin" ./build.sh
batch-git --version
```

文档中的命令统一使用完整名称 `batch-git`。如果希望使用 `bit`，需自行配置：

```sh
alias bit='batch-git'
```

## 3. 建立工作区

### 3.1 扫描已有仓库

在多仓库目录中运行：

```sh
batch-git scan
batch-git scan --depth 2
```

`scan` 创建或增量补充 `workspace.toml`，不会删除已有登记。默认扫描深度为 `1`，
可用 `--depth` 或 `BATCH_GIT_SCAN_DEPTH` 调整。

### 3.2 克隆并登记

```sh
batch-git clone git@example.com:team/service-api.git
batch-git clone -b develop --depth 10 --single-branch \
  git@example.com:team/service-web.git services/service-web
```

克隆成功后才会写入清单。目标目录必须是工作区内的相对路径且不能已经存在。
`batch-git clone` 是系统 `git clone` 的受控代理：上述选项会映射到原生 Git，
认证、credential helper、SSH 配置和代理设置也沿用用户现有的 Git 配置。

### 3.3 从清单恢复

```sh
cd /path/to/restored-workspace
batch-git restore
batch-git fetch
```

`restore` 只恢复缺失仓库；已有仓库不会被覆盖。初始克隆默认分支，随后可用
`fetch` 获取完整的远端引用。

## 4. 查看工作区

```sh
batch-git list
batch-git list --json
batch-git branch
batch-git status
batch-git info
batch-git info service-api
batch-git info services/service-api --json
```

- `list`：登记名称、当前分支和默认分支；
- `branch`：每个仓库的实时当前分支，不访问网络；
- `status`：工作树状态、变更数量和基于本地引用计算的 upstream 差异；
- `info`：工作区元数据，或指定仓库的清单与实时 Git 信息。

`status` 不列出文件名。查看具体文件时使用：

```sh
batch-git -- status --short
```

`info`、`list --json`、`find --json` 和 schedule 的 JSON 输出适合脚本使用。
HTTP(S) 远端 URL 中的用户名、密码或 token 会在保存和展示前移除。

## 5. 更新仓库

### 5.1 fetch

```sh
batch-git fetch
```

对所有 remote 执行 fetch/prune，不 pull、不合并、不切换分支，也不修改工作树。

### 5.2 sync

```sh
batch-git sync
batch-git sync service-api service-web
batch-git sync --match 'service-*'
batch-git sync --all
```

`sync` 是适合无人值守运行的 `restore + fetch`：先恢复选中的缺失仓库，再更新
远端引用。未给选择条件时默认整个工作区；`--all` 用于显式表达相同范围。

### 5.3 pull

```sh
batch-git pull
batch-git pull service-api
batch-git pull --match 'service-*'
```

`pull` 固定使用 fast-forward-only。仓库必须已物化、当前 HEAD 是本地分支、
工作树干净、分支配置了 upstream，且远端更新可以 fast-forward。程序不会自动
stash、rebase、reset 或处理分叉。

### 5.4 push

```sh
batch-git push
batch-git push service-api service-web
batch-git push --match 'service-*'
batch-git push --dry-run
batch-git push --set-upstream
batch-git push -u --remote origin
```

`push` 只推送选中仓库的当前分支，不推送其他分支或 tag，也不提供 force push。
未给选择条件时默认整个工作区。已有 upstream 的分支会推送到其配置的远端分支；
up-to-date 或仅落后 upstream 的分支正常跳过，发生分叉时失败。

没有 upstream 的分支默认跳过，不创建远端分支。只有显式使用
`-u` / `--set-upstream` 时，才会推送到仓库的 primary remote 并建立 tracking；
此时可用 `--remote` 选择其他远端。`--dry-run` 只预览，不修改远端或 upstream。
工作树中的未提交内容不会被推送，但也不会阻止已提交内容执行 push。

清单中的 `push_url` 是独立推送地址的声明来源。字段省略或与 `fetch_url` 相同时，
同步配置会清除仓库中遗留的独立 push URL，push 将回退到 fetch URL。

## 6. 分支操作

### 6.1 搜索分支

```sh
batch-git find main
batch-git find 'feature/*'
batch-git find '*login*' --remote
batch-git find 'release/*' --repo 'service-*'
batch-git find '*' --local --json
```

`*` 匹配零个或多个字符；没有 `*` 时为精确匹配，且匹配区分大小写。远端结果
来自本地已有的 remote-tracking 引用，需要最新结果时先运行 `fetch`。

### 6.2 切换已有分支

```sh
batch-git checkout feature/login
batch-git checkout feature/login --remote origin
batch-git cd
batch-git checkout --default
```

普通 checkout 优先使用本地分支；本地不存在时，从唯一同名远端分支建立
tracking branch。多个远端存在同名分支时必须使用 `--remote` 或
`BATCH_GIT_REMOTE` 消除歧义。没有目标分支的仓库会正常跳过。

`cd` 等价于 `checkout --default`，分别切换到每个仓库清单中的默认分支。它不
fetch、不 pull，也不会强制覆盖工作树。

### 6.3 当前特性分支

```sh
export CURRENT_FEATURE_BRANCH='feature/login'
batch-git cf
# 等价：batch-git checkout --feature
```

变量未设置或为空时，命令给出提示并以成功状态结束，不读取工作区。

### 6.4 创建分支

```sh
batch-git checkout -b feature/new-api
batch-git checkout -b release/2.0 --from main
batch-git checkout -b hotfix --from release --remote origin
```

默认从各仓库当前 HEAD 创建；`--from` 可指向本地分支、唯一远端分支、tag 或
commit。新分支已存在、起点不存在或歧义、工作树阻止安全切换时，该仓库失败。
不支持 `-B` 强制重建。

### 6.5 合并

```sh
batch-git merge feature/login
batch-git merge --feature
batch-git merge --update-current feature/login
batch-git merge --no-update-current feature/login
```

`merge` 将源分支合并到各仓库的当前分支。默认不联网；`--update-current` 会先对
当前 tracking 分支执行 fast-forward-only pull。发生冲突时程序不会自动
`git merge --abort`，应进入对应仓库检查并人工处理。
`merge --feature` 使用 `CURRENT_FEATURE_BRANCH` 作为源分支，无需再传分支名。

## 7. 执行原生 Git

所有已物化仓库：

```sh
batch-git -- status --short
batch-git -- log -1 --oneline
```

指定仓库：

```sh
batch-git exec service-api -- status
batch-git exec service-api service-web -- pull --ff-only
batch-git exec --match 'service-*' -- fetch --prune
```

`exec` 的仓库位置参数按名称或相对目录精确选择；多个选择条件取并集并去重。
`--` 必须存在，其后的参数不经过 shell 解析，直接传给系统 Git。

默认情况下，全仓库透传会展示成功命令的输出；`exec` 只展示失败输出。全局
`--verbose` 可让 `exec` 展示成功输出。需要把全局选项用于透传时，必须放在
分隔符之前：

```sh
batch-git --jobs 8 --verbose -- status
batch-git --jobs 1 exec service-api -- rebase -i HEAD~3
```

并发子进程不接收标准输入；Git 命令需要交互时使用 `--jobs 1`。在交互终端中，
单任务透传会把标准输入、输出和错误流直接交给 Git。透传
`git commit` 遇到明确的 “nothing to commit” 时记为 `skipped`，不会导致聚合失败。

## 8. 登记管理

```sh
batch-git forget service-old
batch-git forget services/legacy
```

`forget` 只删除 `workspace.toml` 中的登记，不删除仓库目录或任何 Git 数据。

## 9. 并发、输出和颜色

多仓库任务使用有上限的并发，但结果始终按清单顺序输出。单仓库失败不会取消
其他仓库。并发数优先级为：`--jobs` > `BATCH_GIT_JOBS` > `4`。

交互终端中的 clone、restore 和 fetch 会显示动态进度；重定向或 CI 中自动退化为
稳定表格。设置 `NO_COLOR`、`TERM=dumb`，或将输出接入管道时，不输出颜色控制码。

## 10. 退出码

| 退出码 | 含义 |
|---:|---|
| `0` | 命令完成；允许预期内的 checkout skip |
| `1` | 至少一个仓库操作失败 |
| `2` | 参数、配置、工作区、清单校验或文件写入错误 |

批量命令可能部分成功。看到退出码 `1` 时，应以结果表中的仓库级状态为准。

## 11. 常见问题

### 找不到 `workspace.toml`

确认当前目录位于工作区内，或设置绝对路径：

```sh
export BATCH_GIT_WORKSPACE='/absolute/path/to/workspace'
batch-git info
```

### 搜索不到刚创建的远端分支

`find --remote` 不联网。先运行 `batch-git fetch` 再搜索。

### checkout 报远端分支歧义

```sh
batch-git checkout feature/login --remote origin
```

也可设置 `BATCH_GIT_REMOTE=origin`。

### pull 失败但 fetch 正常

`pull` 的安全条件更严格。用 `batch-git status` 检查 dirty、detached、upstream 和
分叉状态，再进入失败仓库处理。

### push 跳过没有 upstream 的分支

默认不会自动创建远端分支。确认需要首次推送后执行：

```sh
batch-git push --set-upstream
# 或指定远端
batch-git push -u --remote origin
```

### 命令看起来一直在等待

会执行 Git 或写入清单的进程使用 `.workspace.lock` 串行化。检查是否已有
batch-git 任务或设置为 `queue` 的定时任务正在运行。
