//! Stable top-level error descriptors and diagnostic sanitization.

use crate::error::{ClassifiedError, ErrorCode};

pub(super) struct ErrorDescriptor {
    pub(super) code: &'static str,
    pub(super) retryable: bool,
    pub(super) hint: Option<&'static str>,
}

pub(super) fn error_descriptor(error: &anyhow::Error) -> ErrorDescriptor {
    let code = error
        .downcast_ref::<ClassifiedError>()
        .map(ClassifiedError::code);
    match code {
        Some(ErrorCode::StaleWorkspaceRevision) => ErrorDescriptor {
            code: "stale_workspace_revision",
            retryable: true,
            hint: Some("Run the plan again and apply the new workspace revision."),
        },
        Some(ErrorCode::InvalidArguments) => ErrorDescriptor {
            code: "invalid_arguments",
            retryable: false,
            hint: None,
        },
        Some(ErrorCode::UnknownRepository) => ErrorDescriptor {
            code: "unknown_repository",
            retryable: false,
            hint: Some("Use list --output json to inspect canonical repository names."),
        },
        Some(ErrorCode::AmbiguousRepository) => ErrorDescriptor {
            code: "ambiguous_repository",
            retryable: false,
            hint: Some("Use a canonical repository name or workspace-relative directory."),
        },
        Some(ErrorCode::SelectorNoMatch) => ErrorDescriptor {
            code: "selector_no_match",
            retryable: false,
            hint: Some("Inspect names with list --output json before retrying."),
        },
        Some(ErrorCode::WorkspaceNotFound) => ErrorDescriptor {
            code: "workspace_not_found",
            retryable: false,
            hint: Some("Run scan or set BATCH_GIT_WORKSPACE to an existing workspace."),
        },
        Some(ErrorCode::WorkspaceManifestInvalid) => ErrorDescriptor {
            code: "workspace_manifest_invalid",
            retryable: false,
            hint: Some("Fix batchspace.toml and retry."),
        },
        Some(ErrorCode::Timeout) => ErrorDescriptor {
            code: "timeout",
            retryable: true,
            hint: Some("Retry with a larger --timeout after checking connectivity."),
        },
        Some(ErrorCode::WorkspaceLocked) => ErrorDescriptor {
            code: "workspace_locked",
            retryable: true,
            hint: Some("Wait for the other batch-git operation to finish and retry."),
        },
        #[cfg(feature = "schedule")]
        Some(ErrorCode::ScheduleInvalid) => ErrorDescriptor {
            code: "schedule_invalid",
            retryable: false,
            hint: None,
        },
        None => ErrorDescriptor {
            code: "operation_failed",
            retryable: false,
            hint: None,
        },
    }
}

/// Remove URL user-info before including a diagnostic in a machine-readable protocol.
pub(crate) fn sanitize_message(message: &str) -> String {
    let mut result = String::with_capacity(message.len());
    let mut remainder = message;
    while let Some(scheme_index) = remainder.find("://") {
        let (prefix, after_prefix) = remainder.split_at(scheme_index + 3);
        result.push_str(prefix);
        let end = after_prefix
            .find(|character: char| {
                character.is_whitespace() || matches!(character, '/' | '\\' | '"' | '\'' | ')')
            })
            .unwrap_or(after_prefix.len());
        let (authority, tail) = after_prefix.split_at(end);
        if let Some((_, host)) = authority.rsplit_once('@') {
            result.push_str("***@");
            result.push_str(host);
        } else {
            result.push_str(authority);
        }
        remainder = tail;
    }
    result.push_str(remainder);
    result
}

#[cfg(test)]
mod tests {
    use super::error_descriptor;
    use crate::error::{ErrorCode, classified};

    #[test]
    fn ordinary_words_do_not_change_the_error_code() {
        let error = anyhow::anyhow!("failed to schedule a lock timeout report");
        assert_eq!(error_descriptor(&error).code, "operation_failed");
    }

    #[test]
    fn typed_errors_select_the_stable_descriptor() {
        let error = classified(ErrorCode::Timeout, "child did not finish");
        let descriptor = error_descriptor(&error);
        assert_eq!(descriptor.code, "timeout");
        assert!(descriptor.retryable);
    }
}
