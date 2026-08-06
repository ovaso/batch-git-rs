use crate::color;
use crate::git::GitOutput;
use crate::model::RepositoryRecord;

/// 单仓库操作的三态结果；跳过不等同于失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResultKind {
    Success,
    Skipped,
    Failed,
}

impl ResultKind {
    /// 返回稳定、无颜色的短标签。
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Success => "ok",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    /// 为表格输出生成语义化颜色标签。
    pub(super) fn colored_label(self) -> String {
        match self {
            Self::Success => color::green(self.label()),
            Self::Skipped => self.label().to_owned(),
            Self::Failed => color::red(self.label()),
        }
    }

    /// 为详细输出块选择 Unicode 或 ASCII 标记。
    pub(super) fn block_label(self, unicode: bool) -> &'static str {
        match (self, unicode) {
            (Self::Success, true) => "✓ ok",
            (Self::Skipped, true) => "– skipped",
            (Self::Failed, true) => "✗ failed",
            (Self::Success, false) => "ok",
            (Self::Skipped, false) => "skipped",
            (Self::Failed, false) => "failed",
        }
    }

    pub(super) fn colored_block_label(self, unicode: bool) -> String {
        match self {
            Self::Success => color::green(self.block_label(unicode)),
            Self::Skipped => self.block_label(unicode).to_owned(),
            Self::Failed => color::red(self.block_label(unicode)),
        }
    }
}

/// 一个仓库的业务结果、子进程输出和清单更新时间信息。
#[derive(Debug)]
pub(crate) struct RepositoryResult {
    pub(super) name: String,
    pub(super) directory: String,
    pub(super) kind: ResultKind,
    pub(super) detail: String,
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) exit_code: Option<i32>,
    pub(super) synced: bool,
}

impl RepositoryResult {
    /// 构造成功结果，并可标记该仓库已经完成远端同步。
    pub(crate) fn success(
        repository: &RepositoryRecord,
        detail: impl Into<String>,
        marks_synced: bool,
    ) -> Self {
        let mut result = Self::plain(repository, ResultKind::Success, detail);
        result.synced = marks_synced;
        result
    }

    /// 构造预期内跳过结果。
    pub(crate) fn skipped(repository: &RepositoryRecord, detail: impl Into<String>) -> Self {
        Self::plain(repository, ResultKind::Skipped, detail)
    }

    /// 构造没有子进程输出的失败结果。
    pub(crate) fn failed(repository: &RepositoryRecord, detail: impl Into<String>) -> Self {
        Self::plain(repository, ResultKind::Failed, detail)
    }

    /// 保留 Git 输出但把特定非零结果归类为跳过。
    pub(crate) fn skipped_from_git(
        repository: &RepositoryRecord,
        detail: impl Into<String>,
        output: GitOutput,
    ) -> Self {
        Self {
            name: repository.name.clone(),
            directory: repository.directory.clone(),
            kind: ResultKind::Skipped,
            detail: detail.into(),
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: None,
            synced: false,
        }
    }

    /// 按 Git 退出状态构造成功或失败结果。
    pub(crate) fn from_git(
        repository: &RepositoryRecord,
        output: GitOutput,
        success_detail: impl Into<String>,
        marks_synced: bool,
    ) -> Self {
        let kind = if output.success {
            ResultKind::Success
        } else {
            ResultKind::Failed
        };
        let detail = if output.success {
            success_detail.into()
        } else {
            format!("Git exited with status {}", output.code.unwrap_or(-1))
        };
        Self {
            name: repository.name.clone(),
            directory: repository.directory.clone(),
            kind,
            detail,
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: (!output.success).then_some(output.code.unwrap_or(-1)),
            synced: output.success && marks_synced,
        }
    }

    /// 判断调用方是否应更新清单中的 `synced_at`。
    pub(crate) fn was_synced(&self) -> bool {
        self.synced
    }

    /// 判断该结果是否应让聚合退出码变为 1。
    pub(crate) fn is_failed(&self) -> bool {
        self.kind == ResultKind::Failed
    }

    pub(super) fn reason_code(&self) -> Option<&'static str> {
        let detail = self.detail.to_ascii_lowercase();
        if detail.contains("working tree is not clean") {
            Some("dirty_worktree")
        } else if detail.contains("has no upstream") {
            Some("no_upstream")
        } else if detail.contains("branch is ambiguous") {
            Some("branch_ambiguous")
        } else if detail.contains("does not exist") {
            Some("branch_missing")
        } else if detail.contains("not materialized") || detail.contains("not a git repository") {
            Some("repository_unavailable")
        } else if detail.contains("nothing to push") {
            Some("nothing_to_push")
        } else if detail.contains("nothing to stage") {
            Some("nothing_to_stage")
        } else if detail.contains("nothing to unstage") {
            Some("nothing_to_unstage")
        } else if detail.contains("nothing to commit") {
            Some("nothing_to_commit")
        } else if detail.contains("unresolved conflicts") {
            Some("unresolved_conflicts")
        } else if detail.contains("head is detached") {
            Some("detached_head")
        } else if detail.contains("repository operation is in progress") {
            Some("repository_operation_in_progress")
        } else if detail.contains("timed out") {
            Some("timeout")
        } else if self.exit_code.is_some() {
            Some("git_exit")
        } else if self.kind == ResultKind::Failed {
            Some("operation_failed")
        } else {
            None
        }
    }

    /// 生成进度条结束时显示的短状态。
    pub(crate) fn progress_label(&self) -> String {
        match self.kind {
            ResultKind::Success => color::green("done"),
            ResultKind::Skipped => "verified".to_owned(),
            ResultKind::Failed => color::red("failed"),
        }
    }

    /// 构造不带 Git stdout/stderr 的基础结果。
    fn plain(repository: &RepositoryRecord, kind: ResultKind, detail: impl Into<String>) -> Self {
        let detail = crate::automation::sanitize_message(&detail.into());
        Self {
            name: repository.name.clone(),
            directory: repository.directory.clone(),
            kind,
            detail,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            synced: false,
        }
    }
}
