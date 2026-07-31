//! Workspace discovery, locking, and atomic manifest persistence.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::model::{LOCK_FILE, WORKSPACE_FILE, Workspace, now};

/// 通过持有文件句柄维持的进程级工作区独占锁。
pub struct WorkspaceLock {
    // 字段无需读取；生命周期结束并关闭文件时操作系统会自动释放锁。
    _file: File,
}

impl WorkspaceLock {
    /// 阻塞获取工作区锁，适合用户主动发起的写操作。
    pub fn acquire(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)
            .with_context(|| format!("failed to create workspace root {}", root.display()))?;
        let path = root.join(LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        // fs2 在 Unix 和 Windows 上分别映射到对应的原生文件锁机制。
        file.lock_exclusive()
            .with_context(|| format!("failed to lock {}", path.display()))?;
        Ok(Self { _file: file })
    }

    /// 非阻塞尝试获取锁；锁被占用时返回 `None` 而不是错误。
    pub fn try_acquire(root: &Path) -> Result<Option<Self>> {
        fs::create_dir_all(root)
            .with_context(|| format!("failed to create workspace root {}", root.display()))?;
        let path = root.join(LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error).with_context(|| format!("failed to lock {}", path.display())),
        }
    }
}

/// 返回规范化后的当前目录，消除符号链接和相对路径差异。
pub fn current_root() -> Result<PathBuf> {
    std::env::current_dir()
        .context("failed to get current directory")?
        .canonicalize()
        .context("failed to canonicalize current directory")
}

/// 查找工作区根目录，找不到时生成面向用户的错误。
pub fn find_root() -> Result<PathBuf> {
    let root = find_root_optional()?;
    match root.filter(|root| root.join(WORKSPACE_FILE).is_file()) {
        Some(root) => Ok(root),
        None => bail!(
            "no {} found in current directory or its parents",
            WORKSPACE_FILE
        ),
    }
}

/// 优先使用环境变量，否则从当前目录逐层向父目录查找清单。
pub fn find_root_optional() -> Result<Option<PathBuf>> {
    if let Some(path) = workspace_from_env()? {
        return Ok(Some(path));
    }

    let mut current = current_root()?;
    loop {
        if current.join(WORKSPACE_FILE).is_file() {
            return Ok(Some(current));
        }
        if !current.pop() {
            break;
        }
    }
    Ok(None)
}

/// 解析显式工作区环境变量，并要求绝对路径以避免后台任务目录漂移。
fn workspace_from_env() -> Result<Option<PathBuf>> {
    match std::env::var_os("BATCH_GIT_WORKSPACE") {
        Some(value) => {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                bail!("BATCH_GIT_WORKSPACE must be an absolute path");
            }
            Ok(Some(path.canonicalize().with_context(|| {
                format!(
                    "failed to canonicalize BATCH_GIT_WORKSPACE {}",
                    path.display()
                )
            })?))
        }
        None => Ok(None),
    }
}

/// 读取、反序列化并完整校验工作区清单。
pub fn read(root: &Path) -> Result<Workspace> {
    Ok(read_with_revision(root)?.0)
}

/// Read and validate a manifest together with the SHA-256 digest of the exact bytes parsed.
///
/// Plan generation uses this snapshot to ensure its selected repositories and advertised
/// revision describe one coherent manifest version even if another process writes afterward.
pub fn read_with_revision(root: &Path) -> Result<(Workspace, String)> {
    let path = root.join(WORKSPACE_FILE);
    let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let revision = revision_from_bytes(&bytes);
    let content = String::from_utf8(bytes)
        .with_context(|| format!("failed to decode {} as UTF-8", path.display()))?;
    let workspace: Workspace =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    workspace
        .validate()
        .with_context(|| format!("failed to validate {}", path.display()))?;
    Ok((workspace, revision))
}

/// 读取现有清单；文件不存在时创建内存中的空工作区。
pub fn read_or_new(root: &Path) -> Result<Workspace> {
    if root.join(WORKSPACE_FILE).is_file() {
        read(root)
    } else {
        Ok(Workspace::new())
    }
}

/// Return a stable digest of the current manifest bytes for plan/apply preconditions.
pub fn revision(root: &Path) -> Result<String> {
    let path = root.join(WORKSPACE_FILE);
    let content = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(revision_from_bytes(&content))
}

fn revision_from_bytes(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    format!("sha256:{digest:x}")
}

/// Reject an apply operation when the manifest changed after its plan was generated.
pub fn verify_revision(root: &Path, expected: &str) -> Result<()> {
    let actual = revision(root)?;
    if actual != expected {
        bail!("workspace revision changed; expected {expected}, found {actual}");
    }
    Ok(())
}

/// 更新时间、稳定排序并通过临时文件原子替换清单。
pub fn write(root: &Path, workspace: &mut Workspace) -> Result<()> {
    workspace.updated_at = now();
    workspace
        .repositories
        .sort_by(|a, b| a.directory.cmp(&b.directory));
    workspace.validate()?;
    let content = toml::to_string_pretty(workspace).context("failed to encode workspace.toml")?;
    // 临时文件必须与目标同目录，才能依赖同一文件系统上的原子 rename。
    let mut temporary = NamedTempFile::new_in(root)
        .with_context(|| format!("failed to create temporary file in {}", root.display()))?;
    temporary
        .write_all(content.as_bytes())
        .context("failed to write temporary workspace file")?;
    temporary
        .as_file_mut()
        .sync_all()
        .context("failed to sync temporary workspace file")?;
    let destination = root.join(WORKSPACE_FILE);
    temporary
        .persist(&destination)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", destination.display()))?;
    // 尽力同步目录项；部分平台不支持目录 fsync，因此失败不覆盖已成功的持久化。
    File::open(root)
        .and_then(|directory| directory.sync_all())
        .ok();
    Ok(())
}
