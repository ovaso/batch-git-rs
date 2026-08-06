//! Manifest membership changes that never delete repository data.

use std::collections::HashSet;

use anyhow::Result;
use serde_json::json;

use super::super::verify_apply_revision;
use crate::automation::{self, AutomationOptions};
use crate::cli::ForgetArgs;
use crate::error::{ErrorCode, classified};
use crate::workspace::{self, WorkspaceLock};

/// Remove declarations only; never remove repository directories or Git data.
pub(in crate::commands) fn forget(
    arguments: ForgetArgs,
    automation: &AutomationOptions,
) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    verify_apply_revision(&root, automation)?;
    let mut manifest = workspace::read(&root)?;
    let mut indexes = HashSet::new();
    for selector in &arguments.selectors {
        let matches = manifest
            .repositories
            .iter()
            .enumerate()
            .filter_map(|(index, repository)| {
                (repository.name == *selector || repository.directory == *selector).then_some(index)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [index] => {
                indexes.insert(*index);
            }
            [] => {
                return Err(classified(
                    ErrorCode::UnknownRepository,
                    format!("unknown repository selector: {selector}"),
                ));
            }
            _ => {
                return Err(classified(
                    ErrorCode::AmbiguousRepository,
                    format!("ambiguous repository selector: {selector}"),
                ));
            }
        }
    }
    let removed = manifest
        .repositories
        .iter()
        .enumerate()
        .filter(|(index, _)| indexes.contains(index))
        .map(|(_, repository)| repository.name.clone())
        .collect::<Vec<_>>();
    manifest.repositories = manifest
        .repositories
        .into_iter()
        .enumerate()
        .filter_map(|(index, repository)| (!indexes.contains(&index)).then_some(repository))
        .collect();
    workspace::write(&root, &mut manifest)?;
    if automation.is_machine() {
        automation::emit_data(
            automation,
            "forget",
            Some(&root),
            0,
            &json!({"removed": removed, "directories_deleted": false}),
        )?;
    } else {
        for name in removed {
            println!("forgot {name}; repository directory was not deleted");
        }
    }
    Ok(0)
}
