//! Versioned machine-output protocol and agent execution options.

mod error;
mod options;
mod output;

pub(crate) use error::sanitize_message;
pub(crate) use options::{AutomationOptions, OutputFormat, parse_timeout};
pub(crate) use output::{
    API_VERSION, emit_data, emit_data_with_workspace_revision, emit_error, emit_event,
    emit_finished,
};
