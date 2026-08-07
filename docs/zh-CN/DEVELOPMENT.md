# 开发与发布说明

[English](../DEVELOPMENT.md)

## 1. 技术边界

- `git2/libgit2`：本地仓库读取、checkout、远端和分支配置，不启用网络特性；
- 系统 Git CLI：add、commit、restore/read-tree unstage、clone、fetch、push、merge、pull
  以及 `--` 后的精确透传；
- `workspace.toml`：声明式、可复制的工作区清单；
- `.workspace.lock`：串行化可能修改 Git 状态或清单的 batch-git 进程；
- `automation result v1`：全局 `--output json|jsonl` 的稳定协议；旧子命令 `--json` 只作兼容；
- Rayon：有界并发和稳定结果顺序；
- clap：严格的内建命令边界和命令帮助。

禁止隐式执行会改变提交关系或丢失工作树数据的动作。除非用户明确调用对应命令，
不自动 pull、merge、rebase、stash、reset、clean 或切换分支。
内建 `commit` 只读取既有 index，不得增加 implicit add、amend、空提交或 hook bypass；应拒绝
detached HEAD、未解决冲突和进行中的 Git operation。内建 `add` 首版固定为全部非忽略新增、
修改和删除且拒绝冲突；`unstage` 必须只改 index、保留工作树，unborn HEAD 使用
`git read-tree --empty`，不得引入第二份持久化状态。

## 2. 本地验证

提交或封版前执行：

```sh
cargo fmt -- --check
cargo test --locked
cargo check --locked --no-default-features
cargo test --locked --no-default-features
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo build --locked --release
cargo build --locked --release --no-default-features
./target/release/batch-git --help
./target/release/batch-git add --help
./target/release/batch-git commit --help
./target/release/batch-git unstage --help
./target/release/batch-git env --help
./target/release/batch-git schedule --help
```

默认 feature `schedule` 保持 GitHub Release 构建使用的命令面不变；`--no-default-features`
用于验证精简构建。
该构建必须继续读取和原样保留清单中的 schedule 声明，但 `capabilities.commands` 和顶层帮助中
不得宣称存在未编译的 `schedule` 命令。三平台 artifact 生成器在启用 feature 时都参与编译和
测试，以保留跨平台预览；只允许真实宿主系统调用使用 `target_os` 条件编译。

发布前还应运行 `cargo deny check advisories bans licenses sources`。GitHub Actions 会在
Linux 上分别对默认与无默认 features 执行完整质量门禁，并在 Linux、macOS 与 Windows 上构建
release 产物；它还会生成可下载的 LCOV 覆盖率报告。tag `v*` 触发带补全和许可证的 GitHub
Release 归档、逐文件及统一 SHA-256、安装器和 GitHub build provenance attestation。

受保护的 `main` 只通过 pull request 接收变更。版本号与 changelog 修改必须先合并并通过必需的
CI 检查，之后才能创建 release tag。Release workflow 会在启动平台矩阵前验证 `v*` tag 已存在
于远端；手动触发只用于重试已有 tag。同一 tag 的运行会串行执行，避免并发触发争抢发布同一个
GitHub Release。

发布构建还应检查动态依赖，确保没有意外链接本机构建环境中的 Homebrew、包管理器
或其他非系统绝对路径。macOS 可使用 `otool -L target/release/batch-git`，Linux 可
使用 `ldd target/release/batch-git`。CI 与 Release 的平台矩阵会把检查结果记录在 job 日志中。

`tests/mvp.rs` 覆盖主要端到端工作流；add/commit/unstage 变更应覆盖 index、工作树、unborn、
冲突、detached HEAD、hook 和部分成功边界。各模块内单元测试覆盖解析、清单校验、输出和平台
定义生成。`tests/automation_protocol.rs` 覆盖 v1 receipt、旧 JSON 兼容、结构化参数错误、JSONL
生命周期、暂存命令 reason code / capabilities / plan parameters，以及 plan/apply 在加锁前后都
生效的 revision 前置条件。

## 3. 文档维护约定

- `README.md` 是英文项目入口，`README.zh-CN.md` 是简体中文镜像；两者只保留定位、安装、快速开始和文档入口；
- 英文专题文档位于 `docs/`，同步的简体中文翻译位于 `docs/zh-CN/`；
- 用户行为写入 `docs/USER_GUIDE.md` 及中文镜像；
- 清单和环境变量写入 `docs/WORKSPACE.md` 及中文镜像；
- schedule 平台细节写入 `docs/SCHEDULES.md` 及中文镜像；
- 版本交付内容写入 `CHANGELOG.md` 与 `CHANGELOG.zh-CN.md`；
- 机器可读输出变更须同步维护 `AUTOMATION_CONTRACTS.md`，新增字段可以向后兼容，
  删除或改变字段类型必须在下一个主版本进行；
- 修改 `--output`、`--request-id`、`--non-interactive`、`--timeout`、`--plan` / `--apply`、
  reason code、JSONL 事件或 `schema` 时，必须更新 automation contracts、用户手册、能力名册
  和仓库内 skill，并增加黑盒协议测试；
- 公开行为或安全边界变化须写入两份 CHANGELOG 的 `Unreleased`；
- 不要仅靠注释解释跨模块决策；将持久化格式、锁、并发或系统 Git 边界的变更同步至
  `ARCHITECTURE.md`。

新增或修改 CLI 参数时，应同步检查：

1. clap 的 `--help` 文案；
2. README 常用命令表；
3. 对应专题手册的中英文版本；
4. 环境变量表和清单示例；
5. 两份 CHANGELOG。

## 4. 封版检查清单

- [ ] `Cargo.toml` 版本与 `CHANGELOG.md` 一致；
- [ ] 格式、测试、Clippy 和 release build 全部通过；
- [ ] `batch-git --help` 与用户手册一致；
- [ ] `workspace.toml` 示例可被当前 schema 读取；
- [ ] macOS/Linux/Windows schedule 限制写明；
- [ ] 没有在文档中把 `bit` 描述为自动安装的命令；
- [ ] 已知限制已记录；
- [ ] 发布产物执行 `batch-git --version` 正确。
- [ ] 发布产物动态依赖不包含构建机私有或包管理器绝对路径。
- [ ] `cargo deny check advisories bans licenses sources` 通过；
- [ ] CI 的 Linux、macOS、Windows release 构建均通过；
- [ ] release 提交通过 pull request 进入 `main`，且所有必需检查均通过；
- [ ] 远端 `v*` tag 精确指向该合并后的 release 提交；
- [ ] tag、GitHub Release、二进制名与 SHA-256 校验和相互对应；
- [ ] `gh attestation verify <archive> --repo ovaso/batch-git-rs` 能验证发布归档；
- [ ] `sh -n install.sh`、PowerShell parser 和四种补全文件检查通过；
- [ ] 中英文文档链接有效且翻译保持同步；
- [ ] `SECURITY.md`、`COMPATIBILITY.md` 和 automation contracts 仍与行为一致。

## 5. 构建和安装脚本

```sh
BATCH_GIT_INSTALL_PATH="$HOME/.local/bin" ./build.sh
```

脚本执行 release 构建，按需创建安装目录，并以可执行权限安装为
`$BATCH_GIT_INSTALL_PATH/batch-git`。未设置安装目录或目标存在但不是目录时会失败。

`install.sh` / `install.ps1` 面向已发布的预编译归档：必须显式指定 tag，下载归档及其同名
`.sha256`，在用户可写前缀中安装二进制和补全，不会请求管理员权限。release workflow 将
`LICENSE`、`README.md`、`README.zh-CN.md` 与 `completions/` 一并打包，并对最终归档签发 provenance；安装器本身
只负责 SHA-256，强 provenance 验证由用户或 CI 通过 GitHub CLI单独执行。
