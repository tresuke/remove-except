//! Error classification for invalid roots and keep patterns.

use std::fmt;

/// Marks an error caused by an invalid processing root or invalid keep patterns.
///
/// [`crate::build_plan`] returns [`anyhow::Error`] values. When the failure was caused by
/// the caller's input rather than by filesystem traversal, the error carries a
/// `ValidationError`, which can be detected with [`anyhow::Error::downcast_ref`]:
///
/// ```
/// use remove_except::{ValidationError, build_plan};
///
/// let error = build_plan(std::path::Path::new("."), &[]).unwrap_err();
/// assert!(error.downcast_ref::<ValidationError>().is_some());
/// ```
///
/// Validation errors include missing or empty patterns, invalid glob syntax, patterns that
/// escape the processing root, Windows drive-relative paths, and roots that cannot be
/// resolved, are not directories, or are filesystem roots. Errors while walking beneath a
/// valid root are not validation errors. The message text is not a stable API contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    message: String,
}

impl ValidationError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ValidationError {}

/// Creates an `anyhow::Error` classified as a validation error.
pub(crate) fn invalid(message: impl Into<String>) -> anyhow::Error {
    ValidationError::new(message).into()
}

/// Wraps an underlying error with a validation-error context.
pub(crate) fn invalid_with_source<E>(source: E, message: impl Into<String>) -> anyhow::Error
where
    E: std::error::Error + Send + Sync + 'static,
{
    anyhow::Error::new(source).context(ValidationError::new(message))
}
