//! Error types shared across argus-core.

use thiserror::Error;

/// Convenience alias used by all fallible core APIs.
pub type CoreResult<T> = Result<T, CoreError>;

/// All errors surfaced by the argus-core public API.
///
/// Variant `Display` text is app-owned and safe to forward to the UI — it never
/// contains secrets or untrusted input verbatim. Provider failures in
/// particular never carry `gh`'s stderr or reqwest's text (which holds URLs).
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("not found")]
    NotFound,

    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// `gh` is not installed, or not logged in for the configured host or
    /// account. GitHub is then unavailable; GitLab keeps working.
    #[error("The GitHub CLI (gh) is missing or not logged in.")]
    GithubUnavailable,

    /// No GitLab base URL, or no token in the keychain for it.
    #[error("GitLab is not set up: add its URL and a token in Settings.")]
    GitlabNotConfigured,

    #[error("GitLab rejected the token: check Settings.")]
    GitlabAuth,

    /// A provider call failed. Argus-owned text only.
    #[error("Could not fetch reviews: {0}")]
    Provider(String),

    /// The local database failed. The text is SQLite's own, which never holds
    /// a token (none is ever stored).
    #[error("database error: {0}")]
    Store(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl CoreError {
    /// Whether this means the provider cannot be used at all right now (as
    /// opposed to one call failing), so the UI can mark it unavailable.
    pub fn is_unavailable(&self) -> bool {
        matches!(
            self,
            CoreError::GithubUnavailable | CoreError::GitlabNotConfigured | CoreError::GitlabAuth
        )
    }
}

impl From<rusqlite::Error> for CoreError {
    fn from(e: rusqlite::Error) -> Self {
        CoreError::Store(e.to_string())
    }
}
