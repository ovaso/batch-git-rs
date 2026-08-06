use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::inspect::is_repository;

/// 在给定深度内发现 Git 工作树，并返回稳定排序的路径。
pub fn discover(root: &Path, max_depth: usize) -> Result<Vec<PathBuf>> {
    if max_depth == 0 {
        bail!("scan depth must be at least 1");
    }
    let mut repositories = Vec::new();
    discover_below(root, root, 0, max_depth, &mut repositories)?;
    repositories.sort();
    repositories.dedup();
    Ok(repositories)
}

/// 深度优先扫描目录；发现仓库后不再进入其内部继续搜索。
fn discover_below(
    root: &Path,
    directory: &Path,
    depth: usize,
    max_depth: usize,
    repositories: &mut Vec<PathBuf>,
) -> Result<()> {
    if depth >= max_depth {
        return Ok(());
    }
    let mut children = fs::read_dir(directory)
        .with_context(|| format!("failed to read directory {}", directory.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let file_type = child.file_type()?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let path = child.path();
        if path == root.join("target") || path.file_name().is_some_and(|name| name == ".git") {
            continue;
        }
        if is_repository(&path) {
            repositories.push(path);
        } else {
            discover_below(root, &path, depth + 1, max_depth, repositories)?;
        }
    }
    Ok(())
}
