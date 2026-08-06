# batch-git 用户手册

[English](../USER_GUIDE.md)

本文面向日常使用者。清单字段见 [WORKSPACE.md](WORKSPACE.md)，定时任务的完整
操作说明见 [SCHEDULES.md](SCHEDULES.md)。面向 CI 与 agent 的字段契约见
[AUTOMATION_CONTRACTS.md](AUTOMATION_CONTRACTS.md)。

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

- 系统 Git；
- 使用定时任务时，macOS 需要 `launchd`，Linux 需要 `systemd --user`，Windows
  使用 Task Scheduler。Windows 首版不支持 cron 声明。
- schedule 时区通过 `BATCH_GIT_TZ` 设置；未设置时使用系统时区。

### 2.1 预编译 release

Linux x86_64、macOS x86_64/arm64 和 Windows x86_64 可使用正式 release。安装器要求明确
`vX.Y.Z` 版本、验证 SHA-256、不调用 `sudo`，默认安装到用户目录：

```sh
VERSION=vX.Y.Z
curl -LO "https://github.com/livenv/batch-git/releases/download/$VERSION/install.sh"
sh install.sh --version "$VERSION"
```

```powershell
$Version = "vX.Y.Z"
Invoke-WebRequest "https://github.com/livenv/batch-git/releases/download/$Version/install.ps1" -OutFile install.ps1
.\install.ps1 -Version $Version
```

手工安装时，同时下载同名 `.sha256` 或统一 `SHA256SUMS`。GitHub CLI 可验证 release workflow
签发的构建 provenance：

```sh
gh attestation verify batch-git-<target>.tar.gz --repo livenv/batch-git
```

归档包含 `completions/`。Unix 安装器会安装 Bash、Zsh 和 Fish 补全；如果自定义前缀不在 shell
默认搜索路径中，把 `<prefix>/share/zsh/site-functions` 加入 `fpath`，或直接 source 对应文件。
PowerShell 可 dot-source `<prefix>\share\batch-git\completions\batch-git.ps1`。

### 2.2 从源码构建

源码构建另需 Rust 1.88 或更新工具链：

```sh
cargo install --locked batch-git
# 已安装 cargo-binstall 时，可按 release 元数据选择预编译归档：
cargo binstall batch-git

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
若 clone 失败或超时，目标目录会保留供人工检查，工具不会递归删除其中可能由并发进程写入的内容；
清理或重命名该目录后才能重试同一目标，且它不会被自动登记到清单。

### 3.3 从清单恢复

```sh
cd /path/to/restored-workspace
batch-git restore
batch-git fetch
```

`restore` 只恢复缺失仓库；已有仓库不会被覆盖。初始克隆默认分支，随后可用
`fetch` 获取完整的远端引用。恢复中的 clone 失败或超时时也会保留目标目录，避免自动删除
可能已被其他进程写入的内容；人工检查并处理该目录后才能重试恢复。

## 4. 查看工作区

```sh
batch-git list
batch-git list --json
batch-git branch
batch-git branch --json
batch-git status
batch-git status --json
batch-git info
batch-git info service-api
batch-git info services/service-api --json
batch-git env ls
```

- `list`：登记名称、当前分支和默认分支；
- `branch`：每个仓库的实时当前分支，不访问网络；
- `status`：工作树状态、变更数量和基于本地引用计算的 upstream 差异；
- `info`：工作区元数据，或指定仓库的清单与实时 Git 信息。
- `env ls`：列出支持的环境变量、默认值和本次调用的最终生效值；无需工作区。

`env ls` 默认显示 `VARIABLE`、`DEFAULT` 和 `CURRENT`；增加 `-d` / `--description` 才显示
`DESCRIPTION`。`CURRENT` 列在兼容颜色的交互终端中显示为绿色，并继续遵循 `NO_COLOR`；
重定向或管道输出保持纯文本。`--jobs` 是全局 CLI 参数，因此
`batch-git --jobs 8 env ls` 会把 `BATCH_GIT_JOBS` 的最终值显示为 `8`。

`status` 不列出文件名。查看具体文件时使用：

```sh
batch-git -- status --short
```

旧的 `info`、`list --json`、`find --json`、`status --json`、`branch --json` 和 schedule
JSON 输出适合已有脚本。新自动化使用统一的全局协议，例如
`batch-git --output json status` 或 `batch-git --output json env ls`；它不会输出表格或进度条，
并会包含协议版本、退出码、结构化错误，以及适用命令的工作区 revision。详见
[自动化契约](AUTOMATION_CONTRACTS.md)。
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
batch-git merge --default
batch-git merge --update-current feature/login
batch-git merge --uc --rs --default
batch-git merge --no-update-current feature/login
batch-git merge --no-refresh-source feature/login
```

`merge` 将源分支合并到各仓库的当前分支。除 `merge --default` 的来源刷新默认值外，默认不联网；
如果本地 tracking ref 显示当前分支落后或已分叉，默认模式会在启动 Git merge 前失败，避免留下进行中的
合并；先执行 `batch-git fetch` 以刷新该判断，再执行 `batch-git pull`，或显式使用
`--update-current`。后者会先对当前 tracking 分支执行 fast-forward-only pull；`--uc` 是它的简短别名。
`--refresh-source`（`--rs`）会
fetch 声明远端，并将来源解析为最新 remote-tracking 分支：`--default` 固定使用各仓库的
`primary_remote`，普通 merge 优先使用 `--remote` / `BATCH_GIT_REMOTE`，否则使用
`primary_remote`。它不会移动本地来源分支。两个开关可以组合，例如当前在 `test` 或 `dev` 时执行
`batch-git merge --uc --rs feature/login`，会先 fast-forward 更新当前目标分支，再将最新远端
feature 合入它。当前目标分支没有 upstream（例如仅本地的协作特性分支）时，`--uc` 会安全跳过
预更新，继续合并来源，不会因 `git pull` 的未跟踪分支错误而失败。发生冲突时程序不会自动
`git merge --abort`，应进入对应仓库检查并人工处理。
`merge --feature` 使用
`CURRENT_FEATURE_BRANCH` 作为源分支，无需再传分支名。

`BATCH_GIT_MERGE_DEFAULT_REFRESH_SOURCE` 控制 `merge --default` 是否刷新并合并远端最新的 default，
默认 `true`；`BATCH_GIT_MERGE_FEATURE_UPDATE_CURRENT` 控制 `merge --feature` 是否先 ff-only 更新
当前分支，默认 `false`。它们不是通用的参数映射：前者只影响 `--default`，后者只影响 `--feature`。
命令行优先于对应场景的默认值：`--uc` / `--update-current` 和 `--rs` / `--refresh-source` 显式启用，
`--no-update-current` 和 `--no-refresh-source` 显式关闭。已移除早期的泛化环境变量
`BATCH_GIT_MERGE_UPDATE_CURRENT` 与 `BATCH_GIT_MERGE_REFRESH_SOURCE`。

`merge --default` 分别读取每个仓库在 `workspace.toml` 中声明的 `default_branch`，将其合入
该仓库的当前分支，适合默认分支名称不同或包含多级路径的工作区。解析时优先使用本地同名分支；
本地不存在时，只在该仓库的 `primary_remote` 中寻找 remote-tracking 分支。它不 fetch，且
单独执行 `batch-git fetch` 不会更新已有的本地默认分支；需要远端最新内容时，应先确认本地默认
分支已按预期更新，或使用 `--rs` 直接合并最新的 remote-tracking 默认分支。当前分支就是声明的
默认分支时，该仓库正常跳过。
`--default` 与位置分支、`--feature`、`--remote` 互斥，但可以与 `--update-current` 或
`--no-update-current`、`--refresh-source`、`--no-refresh-source` 组合。

## 7. 暂存与提交

`add`、`commit` 和 `unstage` 使用与 `sync` 相同的仓库选择器：位置参数可写规范仓库名或
workspace 相对目录，`--match` 按仓库名使用区分大小写的 `*` 通配，`--all` 显式选择整个
工作区。省略所有选择条件时也默认整个工作区。首版不接受文件 pathspec；每个选中仓库都使用
命令定义的全量 index 范围。

### 7.1 暂存全部工作树变更

```sh
batch-git add
batch-git add service-api service-web
batch-git add --match 'service-*'
batch-git add --all
```

`add` 在每个选中仓库中暂存全部未被 Git 忽略的新增、修改和删除，等价于从仓库根目录执行
受控的 `git add --all`。它不会用 force 加入 ignored 文件。仓库没有可暂存内容时结果为
`skipped`；存在未解决冲突时，为避免批量命令把冲突文件意外标记为已解决，该仓库会以
`unresolved_conflicts` 失败。

需要在冲突处理时只暂存明确文件，或首版需要按文件暂存时，应进入单个仓库使用原生 Git，或在
精确选择仓库后使用逃生舱，例如：

```sh
batch-git exec service-api -- add -- src/api.rs
```

### 7.2 审阅并提交暂存区

推荐把暂存、审阅和提交保持为三个明确步骤：

```sh
batch-git add --match 'service-*'
batch-git exec --match 'service-*' -- diff --cached --stat
batch-git commit --match 'service-*' -m 'Update generated clients'
```

`commit` 要求非空的 `-m` / `--message`，并在每个有暂存内容的选中仓库中使用同一消息。它只
提交当前 index，不会隐式暂存工作树内容，也不提供 amend、空提交或绕过 hook 的选项。只有
未暂存或 untracked 变更时，该仓库以 `nothing_to_commit` 正常跳过；unborn 分支只要已有暂存
内容即可创建 root commit。

安全批量 commit 会拒绝 detached HEAD、未解决冲突，以及正在进行的 merge、rebase、
cherry-pick、revert 或其他 Git operation。这样不会用一个普通批量命令意外结束已有的 Git
工作流；应进入对应仓库检查，并使用明确的原生 Git continue/abort 流程。

`commit` 仍遵循各仓库的身份、hook、`core.hooksPath` 和签名配置。pre-commit、commit-msg、
签名程序等可能失败、等待交互或产生额外副作用；需要交互时使用文本模式和 `--jobs 1`。机器
输出及 `--non-interactive` 会关闭 Git stdin 和终端凭据提示，但自定义 hook 或签名程序仍可能
直接访问 TTY。

### 7.3 撤销全部暂存

```sh
batch-git unstage
batch-git unstage service-api service-web
batch-git unstage --match 'service-*'
```

`unstage` 把每个选中仓库的全部 index 变更恢复到 HEAD，同时不改任何工作树文件，也不移动
HEAD。新增文件会留在工作树中并重新显示为 untracked，暂存的修改或删除会重新显示为未暂存。
unborn HEAD 没有可恢复的提交，命令会使用 `git read-tree --empty` 清空 index，工作树仍保持
原样。没有暂存内容时结果为 `nothing_to_unstage` 的正常跳过。

撤销暂存会移除当前 index 快照，不能撤销已经创建的 commit。若暂存内容与工作树内容不同，先用
`git diff --cached` 审阅需要保留的 staged 版本。

### 7.4 并发与部分成功

三个命令都持有工作区锁，但会按 `--jobs` 在不同仓库中有界并发。它们不是跨仓库事务：一个
仓库的 hook、签名、index lock 或 Git 命令失败，不会回滚其他仓库已经完成的暂存、撤销暂存或
commit。尤其是 commit 出现退出码 `1` 时，可能已有部分仓库产生新提交，程序不会自动 reset、
amend 或 rebase。

`--timeout` 只终止直接 Git 子进程。hook、签名或 filter 后代可能继续运行；commit 也可能在
更新 ref 后才因后续步骤超时。遇到 timeout 后应逐仓库检查 HEAD、index 和工作树，不要盲目重试。
batch-git 会持续排空 Git stdout/stderr 以避免 pipe 死锁，但每个流最多保留 1 MiB；大输出只保留
开头和结尾并带截断标记。需要完整 `log`、`diff` 或诊断文件时，请在精确仓库中显式重定向到文件。

## 8. 执行原生 Git

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

无人值守时传入 `--non-interactive`，它关闭 stdin 并设置 `GIT_TERMINAL_PROMPT=0`；使用
`--timeout 30s`、`5m` 或 `1h` 为每个系统 Git 子进程设定上限。`--output json` 和
`--output jsonl` 自动采用非交互子进程策略，确保结构化 stdout 不会被 Git 提示污染。
timeout 只终止直接启动的 Git 子进程，不能保证结束其再派生的认证、传输或 helper 进程；
收到 timeout 后仍应检查相关远端或本地目录，不要假设所有后续工作已经停止。

## 9. 登记管理

```sh
batch-git forget service-old
batch-git forget services/legacy
```

`forget` 只删除 `workspace.toml` 中的登记，不删除仓库目录或任何 Git 数据。

## 10. 并发、输出和颜色

多仓库任务使用有上限的并发，但结果始终按清单顺序输出。单仓库失败不会取消
其他仓库。并发数优先级为：`--jobs` > `BATCH_GIT_JOBS` > `4`。

交互终端中的 clone、restore 和 fetch 会显示动态进度；重定向或 CI 中自动退化为
稳定表格。设置 `NO_COLOR`、`TERM=dumb`，或将输出接入管道时，不输出颜色控制码。

### 10.1 可编程输出、plan 与 apply

```sh
batch-git --output json capabilities
batch-git --output json --request-id build-17 sync --match 'service-*'
batch-git --output jsonl fetch

# 复制 receipt.workspace.revision 到下一条命令。
batch-git --output json --plan pull service-api
batch-git --output json --apply --expect-workspace-revision 'sha256:…' pull service-api
```

`--output json` 每次只输出一个 v1 receipt；`--output jsonl` 每行输出一个事件，仓库事件按
清单顺序而非完成时间输出。单仓库 `clone` 同样发送一个 `repository_finished` 事件，失败时以
请求的目标目录标识该结果。`--plan` 不做写入或网络访问，列出实际范围、风险与预期副作用；`--apply` 在执行前核对
`workspace.toml` revision。该核对不能锁定远端、HEAD、index 或工作树状态，因此仍应把 Git 的执行期
检查和仓库级结果当作最终事实。schedule 有独立的 `schedule plan`、`doctor`、`generate` 和
`--dry-run` 流程。

用以下命令从当前二进制发现契约，而非猜测安装版本：

```sh
batch-git --output json capabilities
batch-git schema operation-result
batch-git --output json schema workspace
```

`schema workspace` 输出的 schema 对应 `workspace.toml` 的 JSON 输入表示；`repositories`、
`schedules` 和 schedule 的默认字段可以省略，不必先将默认值补齐。

## 11. 退出码

| 退出码 | 含义 |
|---:|---|
| `0` | 命令完成；允许预期内的 skip，例如没有内容可暂存、提交或撤销暂存 |
| `1` | 至少一个仓库操作失败 |
| `2` | 参数、配置、工作区、清单校验或文件写入错误 |

批量命令可能部分成功。看到退出码 `1` 时，应以结果表中的仓库级状态为准。

## 12. 常见问题

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

`--timeout` 只限制已经启动的直接 Git 子进程，不限制等待 `.workspace.lock` 的时间，也不能
保证结束 Git 再派生的认证、传输或 helper 进程。若需要避免等待锁，应由调用方设置自己的
整体进程超时或在调用前协调任务。
