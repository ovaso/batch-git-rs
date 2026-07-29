//! Shared repository selection for targeted built-in operations.

use std::collections::HashSet;

use anyhow::{Result, bail};

use crate::model::{RepositoryRecord, Workspace};

pub(crate) fn select(
    workspace: &Workspace,
    selectors: &[String],
    patterns: &[String],
    all: bool,
) -> Result<Vec<RepositoryRecord>> {
    if all && (!selectors.is_empty() || !patterns.is_empty()) {
        bail!("--all cannot be combined with repository selectors or --match");
    }
    if all || (selectors.is_empty() && patterns.is_empty()) {
        return Ok(workspace.repositories.clone());
    }

    let mut selected = HashSet::new();
    for selector in selectors {
        let matches = workspace
            .repositories
            .iter()
            .enumerate()
            .filter_map(|(index, repository)| {
                (repository.name == *selector || repository.directory == *selector).then_some(index)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [index] => {
                selected.insert(*index);
            }
            [] => bail!("unknown repository selector: {selector}"),
            _ => bail!("ambiguous repository selector: {selector}"),
        }
    }

    for pattern in patterns {
        let matches = workspace
            .repositories
            .iter()
            .enumerate()
            .filter_map(|(index, repository)| {
                wildcard_matches(pattern, &repository.name).then_some(index)
            })
            .collect::<Vec<_>>();
        if matches.is_empty() {
            bail!("repository pattern matched nothing: {pattern}");
        }
        selected.extend(matches);
    }

    Ok(workspace
        .repositories
        .iter()
        .enumerate()
        .filter_map(|(index, repository)| selected.contains(&index).then_some(repository.clone()))
        .collect())
}

pub(crate) fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let (mut pattern_index, mut value_index) = (0, 0);
    let (mut star, mut retry_value) = (None, 0);
    while value_index < value.len() {
        if pattern_index < pattern.len() && pattern[pattern_index] == value[value_index] {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star = Some(pattern_index);
            pattern_index += 1;
            retry_value = value_index;
        } else if let Some(star_index) = star {
            pattern_index = star_index + 1;
            retry_value += 1;
            value_index = retry_value;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::wildcard_matches;

    #[test]
    fn wildcard_matching_is_literal_except_for_stars() {
        assert!(wildcard_matches("service-*", "service-api"));
        assert!(wildcard_matches("*", "service-api"));
        assert!(!wildcard_matches("service-?", "service-a"));
    }
}
