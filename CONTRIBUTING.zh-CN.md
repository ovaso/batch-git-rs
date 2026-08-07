# 贡献指南

[English](CONTRIBUTING.md)

感谢贡献。提交 issue 或 pull request 前，请先阅读[行为准则](CODE_OF_CONDUCT.zh-CN.md)与
[开发说明](docs/zh-CN/DEVELOPMENT.md)。

## 提交变更

`main` 是受保护分支。包括纯文档和纯 workflow 修改在内的所有仓库变更，都必须从主题分支通过
pull request 提交；正常流程不直接向 `main` 推送提交或发布提交。

1. 使用 fast-forward-only pull 更新本地 `main`，然后创建聚焦单一目的的主题分支。
2. 先用 issue 或 Discussion 讨论会改变 CLI、清单 schema、调度器语义或安全边界的大改动。
3. 每个 PR 聚焦一个目的，说明用户可见行为、平台影响和回滚方式。
4. 为 bug 添加回归测试；为新行为添加成功、失败和安全边界测试。
5. 运行开发说明中的全部质量命令。不要提交 `target/`、凭据、真实私有 remote URL 或本机状态文件。
6. 更新受影响的中英文 README、专题手册、automation contracts 与两份 CHANGELOG 的 `Unreleased`。
7. 推送主题分支并创建 PR，必需的 GitHub Actions 检查与审阅规则全部通过后才合并。

release tag 不能替代 PR。版本号与 changelog 变更应先通过 PR 合并，确认 `main` 全绿后，再在该
合并提交上创建并推送 `v*` tag。手动运行 Release workflow 仅用于重试已经存在于远端的 tag。

## 兼容性承诺

在同一主版本内，已文档化的 CLI、`workspace.toml` version 1、旧 `--json` payload、automation
protocol v1 及公开 schema 只做向后兼容的扩展。破坏性变更必须在 issue/PR 中说明迁移方案，并在
下一主版本发布。

## 审阅重点

审阅会特别检查：多仓库部分成功的报告、锁与并发、路径逃逸、凭据清理、Git 交互行为、Windows/
macOS/Linux 调度器差异，以及文档和帮助文本是否仍一致。
