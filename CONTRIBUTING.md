# 贡献指南

感谢贡献。提交 issue 或 pull request 前，请先阅读 [行为准则](CODE_OF_CONDUCT.md) 与
[开发说明](docs/DEVELOPMENT.md)。

## 提交变更

1. 先用 issue 或 Discussion 讨论会改变 CLI、清单 schema、调度器语义或安全边界的大改动。
2. 每个 PR 聚焦一个目的，说明用户可见行为、平台影响和回滚方式。
3. 为 bug 添加回归测试；为新行为添加成功、失败和安全边界测试。
4. 运行开发说明中的全部质量命令。不要提交 `target/`、凭据、真实私有 remote URL 或本机状态文件。
5. 更新受影响的 README、专题手册、automation contracts 与 `CHANGELOG.md` 的 `Unreleased`。

## 兼容性承诺

在同一主版本内，已文档化的 CLI、`workspace.toml` version 1、旧 `--json` payload、automation
protocol v1 及公开 schema 只做向后兼容的扩展。破坏性变更必须在 issue/PR 中说明迁移方案，并在
下一主版本发布。

## 审阅重点

审阅会特别检查：多仓库部分成功的报告、锁与并发、路径逃逸、凭据清理、Git 交互行为、Windows/
macOS/Linux 调度器差异，以及文档和帮助文本是否仍一致。
