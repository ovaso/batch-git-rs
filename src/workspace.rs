//! Workspace discovery, locking, and atomic manifest persistence.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use tempfile::NamedTempFile;

use crate::model::{LOCK_FILE, WORKSPACE_FILE, Workspace, now};

pub struct WorkspaceLock {
    _file: File,
}

impl WorkspaceLock {
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
        file.lock_exclusive()
            .with_context(|| format!("failed to lock {}", path.display()))?;
        Ok(Self { _file: file })
    }

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

pub fn current_root() -> Result<PathBuf> {
    std::env::current_dir()
        .context("failed to get current directory")?
        .canonicalize()
        .context("failed to canonicalize current directory")
}

pub fn find_root() -> Result<PathBuf> {
    find_root_optional()?.ok_or_else(|| {
        anyhow::anyhow!(
            "no {} found in current directory or its parents",
            WORKSPACE_FILE
        )
    })
}

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

pub fn read(root: &Path) -> Result<Workspace> {
    let path = root.join(WORKSPACE_FILE);
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let workspace: Workspace =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    workspace.validate()?;
    Ok(workspace)
}

pub fn read_or_new(root: &Path) -> Result<Workspace> {
    if root.join(WORKSPACE_FILE).is_file() {
        read(root)
    } else {
        Ok(Workspace::new())
    }
}

pub fn write(root: &Path, workspace: &mut Workspace) -> Result<()> {
    workspace.updated_at = now();
    workspace
        .repositories
        .sort_by(|a, b| a.directory.cmp(&b.directory));
    workspace.validate()?;
    let content = toml::to_string_pretty(workspace).context("failed to encode workspace.toml")?;
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
    File::open(root)
        .and_then(|directory| directory.sync_all())
        .ok();
    Ok(())
}
