# 安全政策

## 支持版本

仅最新发布版本和 `main` 分支接受安全修复。旧版本不回移修复，除非维护者公开说明。

## 报告漏洞

请不要在公开 issue 中披露可利用细节、凭据、私有仓库 URL 或未修复漏洞。请通过 GitHub
Security Advisories 的私密报告功能联系维护者；在该功能尚未启用前，请在 GitHub 上联系
仓库维护者并标明“security report”。请提供复现步骤、受影响版本、影响范围和可行修复。

维护者目标是在 7 天内确认报告，并在可复现后协调修复、测试和披露时间。我们会在修复发布后
的 release notes 与 `CHANGELOG.md` 中记录已解决问题。

## 安全边界

`batch-git` 不保存凭据，也不会在 HTTP(S) remote URL 输出中保留用户信息。machine protocol
默认不包含 Git 子进程原始 stdout/stderr，并会清理顶层诊断中的 HTTP(S) URL user-info。它刻意
不自动执行破坏性 Git 操作。发现可以绕过这些保证、泄露凭据、逃逸 workspace 目录、覆盖未登记
文件或不安全注册系统调度任务的问题，应按本政策报告。
