//! The IPC-facing error type.
//!
//! Every command returns `Result<T, IpcError>`. `IpcError` is a flat,
//! serializable shape so the frontend can branch on a stable `kind` string and
//! show the (already-sanitized) `message`. Messages come from
//! [`CoreError`](argus_core::CoreError)'s `Display` (app-owned text) or from
//! IPC-layer validation here — never from secrets.

use serde::Serialize;

use argus_core::{CoreError, PostError};

/// A flat, serializable error returned by every IPC command.
///
/// `kind` is a stable machine-readable category (camelCase, matching the
/// [`CoreError`] variant, plus a few IPC-only kinds); `message` is a
/// human-readable, secret-free description.
#[derive(Debug, Clone, Serialize)]
pub struct IpcError {
    /// Human-readable, sanitized description (never contains secrets).
    pub message: String,
    /// Stable machine-readable category.
    pub kind: String,
}

impl IpcError {
    /// Build an `IpcError` from a `kind` discriminant and a (secret-free)
    /// message. Used for IPC-layer conditions that have no [`CoreError`]
    /// counterpart.
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
        }
    }

    /// Invalid command input detected at the IPC layer.
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new("invalidInput", message)
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.kind)
    }
}

impl std::error::Error for IpcError {}

impl From<CoreError> for IpcError {
    fn from(err: CoreError) -> Self {
        // The `message` is `CoreError`'s own `Display`, which is built from
        // app-owned text only — safe to forward verbatim.
        let kind = match &err {
            CoreError::NotFound => "notFound",
            CoreError::InvalidInput(_) => "invalidInput",
            CoreError::GithubUnavailable => "githubUnavailable",
            CoreError::GitlabNotConfigured => "gitlabNotConfigured",
            CoreError::GitlabAuth => "gitlabAuth",
            CoreError::Provider(_) => "provider",
            CoreError::Store(_) => "store",
            CoreError::Io(_) => "io",
        };
        Self {
            kind: kind.to_string(),
            message: err.to_string(),
        }
    }
}

impl From<PostError> for IpcError {
    fn from(err: PostError) -> Self {
        // `PostError`'s message is app-owned text, or the platform's own
        // reason already run through the core's `sanitize` (token redacted,
        // one line, capped) — safe to forward verbatim.
        Self {
            kind: err.kind.as_str().to_string(),
            message: err.message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use argus_core::PostErrorKind;

    #[test]
    fn post_errors_keep_their_kind_and_message() {
        for (kind, wire) in [
            (PostErrorKind::PostRejected, "postRejected"),
            (PostErrorKind::LineNotInDiff, "lineNotInDiff"),
            (PostErrorKind::HeadMoved, "headMoved"),
            (PostErrorKind::GitlabAuth, "gitlabAuth"),
            (PostErrorKind::NotFound, "notFound"),
        ] {
            let err = IpcError::from(PostError::new(
                kind,
                "GitHub: line must be part of the diff",
            ));
            assert_eq!(err.kind, wire);
            assert_eq!(err.message, "GitHub: line must be part of the diff");
        }
    }

    #[test]
    fn a_core_error_through_a_post_error_matches_the_direct_mapping() {
        let errors: [fn() -> CoreError; 4] = [
            || CoreError::NotFound,
            || CoreError::GithubUnavailable,
            || CoreError::GitlabNotConfigured,
            || CoreError::Provider("could not reach GitLab".into()),
        ];
        for make in errors {
            let direct = IpcError::from(make());
            let via_post = IpcError::from(PostError::from(make()));
            assert_eq!(
                (direct.kind, direct.message),
                (via_post.kind, via_post.message)
            );
        }
    }
}
