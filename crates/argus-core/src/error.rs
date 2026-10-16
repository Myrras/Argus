//! Error types shared across argus-core.

use thiserror::Error;

/// Convenience alias used by all fallible core APIs.
pub type CoreResult<T> = Result<T, CoreError>;

/// All errors surfaced by the argus-core public API.
///
/// Variant `Display` text is app-owned and safe to forward to the UI — it never
/// contains secrets or untrusted input verbatim.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("note not found")]
    NotFound,

    #[error("invalid input: {0}")]
    InvalidInput(String),
}
