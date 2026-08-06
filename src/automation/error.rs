//! Stable top-level error classification and diagnostic sanitization.

pub(super) struct ErrorDescriptor {
    pub(super) code: &'static str,
    pub(super) retryable: bool,
    pub(super) hint: Option<&'static str>,
}

pub(super) fn error_descriptor(message: &str) -> ErrorDescriptor {
    let lower = message.to_ascii_lowercase();
    if lower.contains("workspace revision changed") {
        ErrorDescriptor {
            code: "stale_workspace_revision",
            retryable: true,
            hint: Some("Run the plan again and apply the new workspace revision."),
        }
    } else if is_invalid_argument_message(&lower) {
        ErrorDescriptor {
            code: "invalid_arguments",
            retryable: false,
            hint: None,
        }
    } else if lower.contains("unknown repository selector") {
        ErrorDescriptor {
            code: "unknown_repository",
            retryable: false,
            hint: Some("Use list --output json to inspect canonical repository names."),
        }
    } else if lower.contains("ambiguous repository selector") {
        ErrorDescriptor {
            code: "ambiguous_repository",
            retryable: false,
            hint: Some("Use a canonical repository name or workspace-relative directory."),
        }
    } else if lower.contains("pattern matched nothing") {
        ErrorDescriptor {
            code: "selector_no_match",
            retryable: false,
            hint: Some("Inspect names with list --output json before retrying."),
        }
    } else if lower.contains("no workspace.toml found") {
        ErrorDescriptor {
            code: "workspace_not_found",
            retryable: false,
            hint: Some("Run scan or set BATCH_GIT_WORKSPACE to an existing workspace."),
        }
    } else if lower.contains("workspace.toml")
        && (lower.contains("parse") || lower.contains("invalid") || lower.contains("validate"))
    {
        ErrorDescriptor {
            code: "workspace_manifest_invalid",
            retryable: false,
            hint: Some("Fix workspace.toml and retry."),
        }
    } else if lower.contains("timed out") {
        ErrorDescriptor {
            code: "timeout",
            retryable: true,
            hint: Some("Retry with a larger --timeout after checking connectivity."),
        }
    } else if lower.contains("lock") {
        ErrorDescriptor {
            code: "workspace_locked",
            retryable: true,
            hint: Some("Wait for the other batch-git operation to finish and retry."),
        }
    } else if lower.contains("schedule") {
        ErrorDescriptor {
            code: "schedule_invalid",
            retryable: false,
            hint: None,
        }
    } else {
        ErrorDescriptor {
            code: "operation_failed",
            retryable: false,
            hint: None,
        }
    }
}

fn is_invalid_argument_message(message: &str) -> bool {
    [
        "invalid arguments",
        "unexpected argument",
        "invalid timeout:",
        "timeout must be",
        "timeout unit must be",
        "timeout is too large",
        "request id must contain",
        "invalid --jobs value",
        "jobs must be at least 1",
        "--jobs requires a value",
        "--output requires a value",
        "--timeout requires a value",
        "--request-id requires a value",
        "--expect-workspace-revision requires a value",
        "invalid output format:",
        "--plan is only valid",
        "--plan conflicts with --apply",
        "--apply is only valid",
        "--apply requires --expect-workspace-revision",
        "--expect-workspace-revision requires --apply",
        "--all cannot be combined with repository selectors or --match",
        "commit message cannot be empty",
        "git passthrough requires arguments after --",
        "use schedule plan ",
    ]
    .iter()
    .any(|marker| message.contains(marker))
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
