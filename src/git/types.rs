use std::sync::Arc;
use std::time::Duration;

use crate::model::RemoteRecord;

/// 系统 Git 子进程的完整、可聚合执行结果。
#[derive(Debug)]
pub struct GitOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// 执行系统 Git 时可由调用方统一控制的交互与时间边界。
///
/// `non_interactive` 优先于 `allow_stdin`：启用后会关闭标准输入并禁止 Git 的终端
/// 凭据提示。`timeout` 仅约束本次 Git 子进程；超时后会终止并回收直接子进程。
#[derive(Debug, Clone, Copy, Default)]
pub struct GitExecutionOptions {
    pub allow_stdin: bool,
    pub non_interactive: bool,
    pub timeout: Option<Duration>,
}

impl GitExecutionOptions {
    /// 从旧调用点的单个 `allow_stdin` 参数构造兼容选项。
    #[allow(dead_code)] // Compatibility wrappers below remain useful to in-module callers.
    pub const fn legacy(allow_stdin: bool) -> Self {
        Self {
            allow_stdin,
            non_interactive: false,
            timeout: None,
        }
    }
}

/// 扫描仓库时写入清单的稳定 Git 元数据。
#[derive(Debug)]
pub struct RepositoryInfo {
    pub default_branch: String,
    pub primary_remote: String,
    pub remotes: Vec<RemoteRecord>,
}

/// 清单仓库在本地文件系统中的实时状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryRuntimeState {
    Available,
    Missing,
    NotGit,
    Bare,
    Error,
}

impl RepositoryRuntimeState {
    /// 返回适合表格和 JSON 输出的稳定标识。
    pub fn label(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Missing => "missing",
            Self::NotGit => "not-git",
            Self::Bare => "bare",
            Self::Error => "error",
        }
    }
}

/// 不访问网络即可取得的仓库运行时摘要。
#[derive(Debug)]
pub struct RepositoryRuntimeInfo {
    pub state: RepositoryRuntimeState,
    pub current_branch: Option<String>,
    pub head: Option<String>,
    pub local_branches: Option<usize>,
    pub remote_branches: Option<usize>,
}

/// 分支来自本地引用还是 remote-tracking 引用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchKind {
    Local,
    Remote,
}

impl BranchKind {
    /// 返回稳定的机器可读类别名。
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

/// 用于 find/list 输出的单个分支摘要。
#[derive(Debug)]
pub struct BranchSummary {
    pub name: String,
    pub kind: BranchKind,
    pub remote: Option<String>,
    pub is_current: bool,
    pub commit: String,
    pub commit_short: String,
    pub commit_time: i64,
}

/// checkout 名称解析后的确定目标或歧义状态。
#[derive(Debug)]
pub enum CheckoutTarget {
    Local,
    Remote(String),
    Missing,
    Ambiguous(Vec<String>),
}

/// 映射到系统 `git clone` 的受控选项。
pub struct CloneOptions<'a> {
    pub remote_name: &'a str,
    pub branch: Option<&'a str>,
    pub depth: Option<usize>,
    pub single_branch: bool,
    pub progress: Option<CloneProgress>,
    #[allow(dead_code)] // Read by the legacy clone_repository wrapper.
    pub allow_stdin: bool,
}

/// clone 进度回调，参数分别为已完成对象数和总对象数。
pub type CloneProgress = Arc<dyn Fn(usize, usize) + Send + Sync>;

/// 按 Git 状态类别聚合的工作树变更数量。
#[derive(Debug, Default)]
pub struct ChangeCounts {
    pub modified: usize,
    pub added: usize,
    pub deleted: usize,
    pub renamed: usize,
    pub untracked: usize,
    pub conflicted: usize,
}

impl ChangeCounts {
    /// 返回所有状态类别的总变更数。
    pub fn total(&self) -> usize {
        self.modified + self.added + self.deleted + self.renamed + self.untracked + self.conflicted
    }

    /// 生成类似 `M2 ?1` 的紧凑人类可读文本。
    pub fn compact(&self) -> String {
        let values = [
            ("M", self.modified),
            ("A", self.added),
            ("D", self.deleted),
            ("R", self.renamed),
            ("?", self.untracked),
            ("U", self.conflicted),
        ]
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(label, count)| format!("{label}{count}"))
        .collect::<Vec<_>>();
        if values.is_empty() {
            "-".to_owned()
        } else {
            values.join(" ")
        }
    }
}

/// 当前分支相对 upstream 的提交关系。
#[derive(Debug)]
pub enum UpstreamSummary {
    UpToDate,
    Ahead(usize),
    Behind(usize),
    Diverged { ahead: usize, behind: usize },
    None,
}

impl UpstreamSummary {
    /// 生成人类可读的 ahead/behind 摘要。
    pub fn label(&self) -> String {
        match self {
            Self::UpToDate => "up-to-date".to_owned(),
            Self::Ahead(count) => format!("ahead {count}"),
            Self::Behind(count) => format!("behind {count}"),
            Self::Diverged { ahead, behind } => format!("ahead {ahead}, behind {behind}"),
            Self::None => "no-upstream".to_owned(),
        }
    }
}

/// `status` 命令所需的仓库级汇总。
#[derive(Debug)]
pub struct RepositoryStatusSummary {
    pub branch: String,
    pub changes: ChangeCounts,
    pub upstream: UpstreamSummary,
}

/// Index and working-tree facts used by the controlled staging commands.
#[derive(Debug, Default)]
pub struct StagingState {
    pub has_staged_changes: bool,
    pub has_worktree_changes: bool,
    pub has_conflicts: bool,
}
