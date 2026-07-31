# 开发与发布说明

## 1. 技术边界

- `git2/libgit2`：本地仓库读取、checkout、远端和分支配置，不启用网络特性；
- 系统 Git CLI：clone、fetch、push、merge、pull 以及 `--` 后的精确透传；
- `workspace.toml`：声明式、可复制的工作区清单；
- `.workspace.lock`：串行化可能修改 Git 状态或清单的 batch-git 进程；
- `automation result v1`：全局 `--output json|jsonl` 的稳定协议；旧子命令 `--json` 只作兼容；
- Rayon：有界并发和稳定结果顺序；
- clap：严格的内建命令边界和命令帮助。

禁止隐式执行会改变提交关系或丢失工作树数据的动作。除非用户明确调用对应命令，
不自动 pull、merge、rebase、stash、reset、clean 或切换分支。

## 2. 本地验证

提交或封版前执行：

```sh
cargo fmt -- --check
cargo test --locked
cargo clippy --all-targets --all-features -- -D warnings
cargo build --locked --release
./target/release/batch-git --help
./target/release/batch-git schedule --help
```

发布前还应运行 `cargo deny check advisories bans licenses sources`。GitHub Actions 会在
Linux 上执行完整质量门禁，并在 Linux、macOS 与 Windows 上构建 release 产物；
它还会生成可下载的 LCOV 覆盖率报告。tag `v*` 触发 GitHub Release 和 SHA-256 校验和生成。

发布构建还应检查动态依赖，确保没有意外链接本机构建环境中的 Homebrew、包管理器
或其他非系统绝对路径。macOS 可使用 `otool -L target/release/batch-git`，Linux 可
使用 `ldd target/release/batch-git`。

`tests/mvp.rs` 覆盖主要端到端工作流；各模块内单元测试覆盖解析、清单校验、
输出和平台定义生成。`tests/automation_protocol.rs` 覆盖 v1 receipt、旧 JSON 兼容、
结构化参数错误、JSONL 生命周期，以及 plan/apply 在加锁前后都生效的 revision 前置条件。

## 3. 文档维护约定

- README 只保留定位、安装、快速开始和文档入口；
- 用户行为写入 `docs/USER_GUIDE.md`；
- 清单和环境变量写入 `docs/WORKSPACE.md`；
- schedule 平台细节写入 `docs/SCHEDULES.md`；
- 版本交付内容写入 `CHANGELOG.md`；
- `Request.md`、`OPTIMISE.md`、`SUGGESTIONS.md` 作为历史设计记录，不应被引用为
  当前行为规范。
- 机器可读输出变更须同步维护 `AUTOMATION_CONTRACTS.md`，新增字段可以向后兼容，
  删除或改变字段类型必须在下一个主版本进行；
- 修改 `--output`、`--request-id`、`--non-interactive`、`--timeout`、`--plan` / `--apply`、
  reason code、JSONL 事件或 `schema` 时，必须更新 automation contracts、用户手册、能力名册
  和仓库内 skill，并增加黑盒协议测试；
- 公开行为或安全边界变化须写入 `CHANGELOG.md` 的 `Unreleased`；
- 不要仅靠注释解释跨模块决策；将持久化格式、锁、并发或系统 Git 边界的变更同步至
  `ARCHITECTURE.md`。

新增或修改 CLI 参数时，应同步检查：

1. clap 的 `--help` 文案；
2. README 常用命令表；
3. 对应专题手册；
4. 环境变量表和清单示例；
5. CHANGELOG。

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
- [ ] tag、GitHub Release、二进制名与 SHA-256 校验和相互对应；
- [ ] `SECURITY.md`、`COMPATIBILITY.md` 和 automation contracts 仍与行为一致。

## 5. 构建和安装脚本

```sh
BATCH_GIT_INSTALL_PATH="$HOME/.local/bin" ./build.sh
```

脚本执行 release 构建，按需创建安装目录，并以可执行权限安装为
`$BATCH_GIT_INSTALL_PATH/batch-git`。未设置安装目录或目标存在但不是目录时会失败。
