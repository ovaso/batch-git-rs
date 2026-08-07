# 安全政策

[English](SECURITY.md)

## 维护范围

本项目不承诺固定支持周期、响应时限、披露时间表或旧版本回移。涉及当前 `main` 分支或带 tag
GitHub Release 的报告，会根据可复现性、影响范围与维护者精力尽力评估。

## 报告漏洞

请不要在公开 issue 中披露可利用细节、凭据、私有仓库 URL 或未修复漏洞。请通过 GitHub
Security Advisories 的私密报告功能联系维护者；在该功能尚未启用前，请在 GitHub 上联系
仓库维护者并标明“security report”。请提供复现步骤、受影响版本、影响范围和可行修复。

## 安全边界

`batch-git` 不保存凭据，也不会在 HTTP(S) remote URL 输出中保留用户信息。machine protocol
默认不包含 Git 子进程原始 stdout/stderr，并会清理顶层诊断中的 HTTP(S) URL user-info。它刻意
不自动执行破坏性 Git 操作。发现可以越过这些文档边界、泄露凭据、逃逸 workspace 目录、覆盖未登记
文件或不安全注册系统调度任务的问题，应按本政策报告。
