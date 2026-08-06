//! Typed top-level failure categories for the stable automation protocol.

use std::fmt;

use anyhow::Result;

/// Stable machine-readable categories. Human-facing context remains in the error message chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorCode {
    InvalidArguments,
    WorkspaceNotFound,
    WorkspaceManifestInvalid,
    UnknownRepository,
    AmbiguousRepository,
    SelectorNoMatch,
    StaleWorkspaceRevision,
    WorkspaceLocked,
    Timeout,
    #[cfg(feature = "schedule")]
    ScheduleInvalid,
}

/// An error carrying a stable category without coupling callers to rendered message text.
#[derive(Debug)]
pub(crate) struct ClassifiedError {
    code: ErrorCode,
    message: String,
}

impl ClassifiedError {
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn code(&self) -> ErrorCode {
        self.code
    }
}

impl fmt::Display for ClassifiedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ClassifiedError {}

/// Preserve an existing classified source; otherwise classify the complete contextual message.
pub(crate) trait ClassifyResult<T> {
    fn classify(self, code: ErrorCode) -> Result<T>;
}

impl<T> ClassifyResult<T> for Result<T> {
    fn classify(self, code: ErrorCode) -> Result<T> {
        self.map_err(|error| {
            if error.downcast_ref::<ClassifiedError>().is_some() {
                error
            } else {
                ClassifiedError::new(code, format!("{error:#}")).into()
            }
        })
    }
}

pub(crate) fn classified(code: ErrorCode, message: impl Into<String>) -> anyhow::Error {
    ClassifiedError::new(code, message).into()
}

pub(crate) fn error_code(error: &anyhow::Error) -> Option<ErrorCode> {
    error
        .downcast_ref::<ClassifiedError>()
        .map(ClassifiedError::code)
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result, bail};

    use super::{ClassifiedError, ClassifyResult, ErrorCode};

    #[test]
    fn classification_survives_anyhow_context() -> Result<()> {
        let error = Err::<(), _>(super::classified(ErrorCode::Timeout, "timed out"))
            .context("running Git")
            .unwrap_err();

        assert_eq!(
            error
                .downcast_ref::<ClassifiedError>()
                .map(|error| error.code()),
            Some(ErrorCode::Timeout)
        );
        Ok(())
    }

    #[test]
    fn outer_classification_does_not_replace_a_specific_inner_code() {
        let inner = Err::<(), _>(super::classified(ErrorCode::Timeout, "timed out"));
        let error = inner.classify(ErrorCode::InvalidArguments).unwrap_err();

        assert_eq!(
            error
                .downcast_ref::<ClassifiedError>()
                .map(|error| error.code()),
            Some(ErrorCode::Timeout)
        );
    }

    #[test]
    fn classification_preserves_rendered_context() {
        fn fails() -> Result<()> {
            bail!("low-level detail")
        }

        let error = fails()
            .context("high-level operation")
            .classify(ErrorCode::InvalidArguments)
            .unwrap_err();
        assert_eq!(
            format!("{error:#}"),
            "high-level operation: low-level detail"
        );
    }
}
