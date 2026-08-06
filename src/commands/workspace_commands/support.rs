//! Shared invariants for workspace repository paths and generated names.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::model::RepositoryRecord;

/// Atomically reserve an empty clone destination without deleting any user-controlled path.
///
/// Git accepts an existing empty destination. Reserving it with `create_dir` closes the
/// check-then-create race while deliberately leaving failed clone contents for manual review.
pub(super) fn reserve_clone_destination(target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("clone destination has no parent: {}", target.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create clone parent {}", parent.display()))?;
    match fs::create_dir(target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            bail!("clone destination already exists: {}", target.display())
        }
        Err(error) => Err(error)
            .with_context(|| format!("failed to reserve clone destination {}", target.display())),
    }
}

/// Convert a workspace-relative path to the manifest's slash-separated representation.
pub(in crate::commands) fn relative_string(path: &Path) -> Result<String> {
    if path.is_absolute() {
        bail!("directory must be relative to the workspace");
    }
    let parts = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    Ok(parts.join("/"))
}

/// Derive a deterministic unique repository name from its directory when names collide.
pub(super) fn unique_name(
    base: &str,
    directory: &str,
    repositories: &[RepositoryRecord],
) -> String {
    let used = repositories
        .iter()
        .map(|repository| repository.name.as_str())
        .collect::<HashSet<_>>();
    if !used.contains(base) {
        return base.to_owned();
    }
    let candidate = directory.replace('/', "-");
    if !used.contains(candidate.as_str()) {
        return candidate;
    }
    for suffix in 2.. {
        let candidate = format!("{}-{suffix}", directory.replace('/', "-"));
        if !used.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!()
}
