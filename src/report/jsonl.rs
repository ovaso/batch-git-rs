use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use serde_json::json;

use crate::automation::{AutomationOptions, OutputFormat};

use super::machine;
use super::result::RepositoryResult;

/// Serializes completion events from concurrent workers without delaying them until aggregation.
///
/// JSONL preserves the v1 manifest-order contract: out-of-order completions are buffered only
/// until their predecessors arrive, then the contiguous finished prefix is emitted immediately.
pub(crate) struct JsonlProgress<'a> {
    automation: &'a AutomationOptions,
    command: String,
    root: &'a Path,
    state: Mutex<JsonlProgressState>,
}

struct JsonlProgressState {
    next_index: usize,
    pending: BTreeMap<usize, serde_json::Value>,
}

impl<'a> JsonlProgress<'a> {
    /// Emit an immediate lifecycle start event when JSONL progress is requested.
    pub(crate) fn new(
        automation: &'a AutomationOptions,
        command: &str,
        root: &'a Path,
        repositories: usize,
    ) -> Result<Option<Self>> {
        if automation.output != OutputFormat::Jsonl {
            return Ok(None);
        }
        crate::automation::emit_event(
            automation,
            "started",
            command,
            Some(root),
            json!({"repositories": repositories}),
        )?;
        Ok(Some(Self {
            automation,
            command: command.to_owned(),
            root,
            state: Mutex::new(JsonlProgressState {
                next_index: 0,
                pending: BTreeMap::new(),
            }),
        }))
    }

    /// Emit a repository result at completion time. Serialization cannot fail for this fixed
    /// record shape; a poisoned stdout lock indicates the process is already unwinding.
    pub(crate) fn repository_finished(&self, index: usize, result: &RepositoryResult) {
        self.repository_finished_value(index, machine::repository_value(result));
    }

    /// Emit a repository-shaped result supplied by a command that does not use `RepositoryResult`
    /// internally (for example, scan candidates before they exist in the manifest).
    pub(crate) fn repository_finished_value(&self, index: usize, result: serde_json::Value) {
        let data = json!({ "repository_index": index, "result": result });
        let mut state = self
            .state
            .lock()
            .expect("JSONL output lock must not be poisoned");
        state.pending.insert(index, data);
        while let Some(data) = {
            let next_index = state.next_index;
            state.pending.remove(&next_index)
        } {
            crate::automation::emit_event(
                self.automation,
                "repository_finished",
                &self.command,
                Some(self.root),
                data,
            )
            .expect("fixed JSONL repository event must serialize");
            state.next_index += 1;
        }
    }
}
