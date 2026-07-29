# 开发与发布说明

## 1. 技术边界

- `git2/libgit2`：仓库读取、clone、fetch/prune、checkout、远端和分支配置；
- 系统 Git CLI：merge、pull 以及 `--` 后的精确透传；
- `workspace.toml`：声明式、可复制的工作区清单；
- `.workspace.lock`：串行化可能修改 Git 状态或清单的 batch-git 进程；
- Rayon：有界并发和稳定结果顺序；
- clap：严格的内建命令边界和命令帮助。

禁止隐式执行会改变提交关系或丢失工作树数据的动作。除非用户明确调用对应命令，
不自动 pull、merge、rebase、stash、reset、clean 或切换分支。

## 2. 本地验证

提交或封版前执行：

```sh
cargo fmt -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
./target/release/batch-git --help
./target/release/batch-git schedule --help
```

`tests/mvp.rs` 覆盖主要端到端工作流；各模块内单元测试覆盖解析、清单校验、
输出和平台定义生成。

## 3. 文档维护约定

- README 只保留定位、安装、快速开始和文档入口；
- 用户行为写入 `docs/USER_GUIDE.md`；
- 清单和环境变量写入 `docs/WORKSPACE.md`；
- schedule 平台细节写入 `docs/SCHEDULES.md`；
- 版本交付内容写入 `CHANGELOG.md`；
- `Request.md`、`OPTIMISE.md`、`SUGGESTIONS.md` 作为历史设计记录，不应被引用为
  当前行为规范。

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
- [ ] macOS/Linux schedule 限制写明；
- [ ] 没有在文档中把 `bit` 描述为自动安装的命令；
- [ ] 已知限制已记录；
- [ ] 发布产物执行 `batch-git --version` 正确。

## 5. 构建和安装脚本

```sh
BATCH_GIT_INSTALL_PATH="$HOME/.local/bin" ./build.sh
```

脚本执行 release 构建，按需创建安装目录，并以可执行权限安装为
`$BATCH_GIT_INSTALL_PATH/batch-git`。未设置安装目录或目标存在但不是目录时会失败。
