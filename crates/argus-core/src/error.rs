//! Error types shared across argus-core.

use thiserror::Error;

/// Convenience alias used by all fallible core APIs.
pub type CoreResult<T> = Result<T, CoreError>;

/// All errors surfaced by the argus-core public API.
///
/// Variant `Display` text is app-owned and safe to forward to the UI — it never
/// contains secrets or untrusted input verbatim. Provider failures in
/// particular never carry `gh`'s or `glab`'s output.
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

    /// No GitLab instance to talk to (a host that names none).
    #[error("GitLab is not set up: log in with glab and check Settings → Accounts.")]
    GitlabNotConfigured,

    /// The `glab` CLI is not installed (or not where Argus looked).
    #[error("The GitLab CLI (glab) was not found: install it and run `glab auth login`.")]
    GlabMissing,

    /// glab has no login for `host`, or GitLab did not accept it (a 401 or
    /// 403). `host` is the user's own setting, never a secret.
    #[error("Not logged in to {host}: run `glab auth login --hostname {host}`.")]
    GitlabAuth { host: String },

    /// A provider call failed. Argus-owned text only.
    #[error("Could not fetch reviews: {0}")]
    Provider(String),

    /// The local database failed. The text is SQLite's own, which never holds
    /// a token (none is ever stored).
    #[error("database error: {0}")]
    Store(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A local agent CLI (the AI pass' `claude` / `auggie`) could not do
    /// what was asked: off, not found, failed, or an answer Argus could not
    /// use. Argus-owned text only (never the agent's output).
    #[error("{0}")]
    Agent(String),

    /// Stopped by the user before it finished.
    #[error("cancelled")]
    Cancelled,

    /// The write would replace something that is there; nothing was
    /// written. The UI asks before it tries again.
    #[error("{0}")]
    Conflict(String),
}

impl CoreError {
    /// Whether this means the provider cannot be used at all right now (as
    /// opposed to one call failing), so the UI can mark it unavailable.
    pub fn is_unavailable(&self) -> bool {
        matches!(
            self,
            CoreError::GithubUnavailable
                | CoreError::GitlabNotConfigured
                | CoreError::GlabMissing
                | CoreError::GitlabAuth { .. }
        )
    }

    /// Whether the platform rate-limited the provider: the message then says
    /// until when ("GitHub is rate limited until 14:05").
    pub fn is_rate_limited(&self) -> bool {
        crate::provider::is_rate_limited(self)
    }
}

impl From<rusqlite::Error> for CoreError {
    fn from(e: rusqlite::Error) -> Self {
        CoreError::Store(e.to_string())
    }
}
