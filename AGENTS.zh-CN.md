# Agent 协作约定

[English](AGENTS.md)

本仓库是安全优先的 Rust CLI。先阅读 `README.zh-CN.md`、`docs/zh-CN/DEVELOPMENT.md`，再按任务需要
阅读 `docs/zh-CN/ARCHITECTURE.md`、`docs/zh-CN/AUTOMATION_CONTRACTS.md` 和仓库内
`skills/batch-git-automation/SKILL.zh-CN.md`。

## 工作规则

- 保持 `batchspace.toml` 是唯一的持久化工作区事实来源；不得引入第二份状态文件。
- 不要使 `batch-git` 隐式执行 merge、rebase、stash、reset、clean、force push 或删除仓库。
- 改动 CLI 参数、退出码、JSON 字段、清单 schema 或 schedule 行为时，同步更新帮助文本、
  对应中英文用户文档、`AUTOMATION_CONTRACTS.md` 与两份 CHANGELOG。
- 修改 v1 receipt、JSONL 事件、`reason_code`、`capabilities`、`schema` 或 plan/apply 语义时，
  同步维护 `skills/batch-git-automation/`，并在 `tests/automation_protocol.rs` 补充黑盒回归。
- 修改平台调度器代码时，保留 launchd、systemd 和 Windows 行为的明确差异；不得把主机路径
  或本地用户名写入测试断言。
- 优先补行为测试而非实现细节测试。网络、文件系统和调度器调用必须有可替代的测试边界。
- 除非用户明确要求，否则不执行有远端副作用的 Git 命令或注册本机 schedule。

## 验证

Rust 代码变更至少执行：

```sh
cargo fmt -- --check
cargo test --locked
cargo clippy --all-targets --all-features -- -D warnings
cargo build --locked --release
```

文档或自动化变更也应检查链接、YAML/TOML 语法，并说明未能在本机覆盖的平台。
