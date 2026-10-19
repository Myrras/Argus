//! `settings.json`: hosts and accounts, polling, AI, local clones.
//!
//! Load is forgiving (a missing or corrupt file is the defaults, a missing
//! field is its default) and save is atomic (temp file + rename), as in
//! Podium. No secrets live here: GitHub's token stays with `gh`, GitLab's in
//! the OS keychain.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::provider::gitlab::normalize_base_url;
use crate::provider::ReviewRef;

pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 180;
/// Floor for the poll interval, so a typo cannot hammer the APIs.
pub const MIN_POLL_INTERVAL_SECS: u64 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub github: GithubSettings,
    pub gitlab: GitlabSettings,
    pub poll_interval_secs: u64,
    pub ai: AiSettings,
    /// Local clone per repository, keyed `host/repo` (e.g.
    /// `github.com/acme/widgets`). Recommended for deeper AI analysis, and
    /// read (never written, never fetched) for faster, exact thread history
    /// and file contents; never required: what it lacks comes from the API.
    pub clone_paths: BTreeMap<String, PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            github: GithubSettings::default(),
            gitlab: GitlabSettings::default(),
            poll_interval_secs: DEFAULT_POLL_INTERVAL_SECS,
            ai: AiSettings::default(),
            clone_paths: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GithubSettings {
    pub enabled: bool,
    pub host: String,
    /// A logged-in `gh` account to use; `None` is `gh`'s active account.
    pub account: Option<String>,
}

impl Default for GithubSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            host: "github.com".into(),
            account: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GitlabSettings {
    /// Instance root, e.g. `https://gitlab.example.com`. Empty = GitLab off.
    pub base_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiSettings {
    pub enabled: bool,
    /// Agent adapter id: `claude` or `auggie`.
    pub adapter: String,
    /// Extra CLI arguments for the agent.
    pub args: Vec<String>,
    /// The agent binary's path; `None` (or blank) finds the adapter's CLI
    /// through the login shell, like `gh`.
    pub binary: Option<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            adapter: "claude".into(),
            args: Vec::new(),
            binary: None,
        }
    }
}

impl Settings {
    /// Read `path`; a missing or corrupt file is the defaults.
    pub fn load(path: &Path) -> Self {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&raw).unwrap_or_else(|e| {
            tracing::warn!("settings file is corrupt, using defaults: {e}");
            Self::default()
        })
    }

    /// Atomically write to `path`: a temp file renamed over the old one, so
    /// a crash mid-write never leaves a torn file.
    pub fn save(&self, path: &Path) -> CoreResult<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| CoreError::InvalidInput(format!("could not encode settings: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Normalise and check: trims hosts, drops a trailing slash from the
    /// GitLab URL, and rejects a GitLab URL that is not an http(s) instance
    /// URL. Plain http is allowed (a VPN); the status flags it.
    pub fn validated(mut self) -> CoreResult<Self> {
        self.github.host = self.github.host.trim().to_string();
        if self.github.host.is_empty() {
            self.github.host = GithubSettings::default().host;
        }
        self.github.account = self
            .github
            .account
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty());
        let url = self.gitlab.base_url.trim();
        self.gitlab.base_url = if url.is_empty() {
            String::new()
        } else {
            normalize_base_url(url).ok_or_else(|| {
                CoreError::InvalidInput(
                    "the GitLab URL must start with https:// or http:// and have no \
                     user, query or fragment"
                        .into(),
                )
            })?
        };
        Ok(self)
    }

    pub fn gitlab_enabled(&self) -> bool {
        !self.gitlab.base_url.trim().is_empty()
    }

    /// The poll interval, never below [`MIN_POLL_INTERVAL_SECS`].
    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_interval_secs.max(MIN_POLL_INTERVAL_SECS))
    }

    /// The local clone for `review`'s repository, if one is set.
    pub fn clone_path(&self, review: &ReviewRef) -> Option<&Path> {
        self.clone_paths
            .get(&format!("{}/{}", review.host, review.repo))
            .map(PathBuf::as_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderId;

    #[test]
    fn defaults() {
        let s = Settings::default();
        assert!(s.github.enabled);
        assert_eq!(s.github.host, "github.com");
        assert_eq!(s.github.account, None);
        assert_eq!(s.gitlab.base_url, "");
        assert!(!s.gitlab_enabled());
        assert_eq!(s.poll_interval(), Duration::from_secs(180));
        assert!(s.ai.enabled);
        assert_eq!(s.ai.adapter, "claude");
        assert!(s.clone_paths.is_empty());
    }

    #[test]
    fn round_trips_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/settings.json");
        let mut s = Settings::default();
        s.gitlab.base_url = "https://gitlab.example.test".into();
        s.poll_interval_secs = 60;
        s.ai.enabled = false;
        s.clone_paths.insert(
            "github.com/acme-fixture/widgets".into(),
            PathBuf::from("/src/widgets"),
        );
        s.save(&path).unwrap();
        assert!(!path.with_extension("json.tmp").exists());
        assert_eq!(Settings::load(&path), s);

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"pollIntervalSecs\": 60"));
        assert!(raw.contains("\"baseUrl\""));
        assert!(!raw.to_lowercase().contains("token"));

        let r = ReviewRef {
            provider: ProviderId::Github,
            host: "github.com".into(),
            repo: "acme-fixture/widgets".into(),
            number: 1,
            url: String::new(),
            title: String::new(),
            author: String::new(),
            updated_at: None,
        };
        assert_eq!(s.clone_path(&r), Some(Path::new("/src/widgets")));
    }

    #[test]
    fn missing_corrupt_and_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default());
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        std::fs::write(&path, r#"{"github":{"account":"work"},"future":1}"#).unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.github.account.as_deref(), Some("work"));
        assert_eq!(s.github.host, "github.com");
        assert_eq!(s.poll_interval_secs, DEFAULT_POLL_INTERVAL_SECS);
    }

    #[test]
    fn a_gitlab_url_validation_refuses_still_loads_the_rest() {
        // Hand-edited: userinfo in the URL. Loading keeps every other setting (the
        // provider list leaves GitLab off and its status says why).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"gitlab":{"baseUrl":"http://someone@gitlab.example.test"},"pollIntervalSecs":60,"github":{"account":"work"}}"#,
        )
        .unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.gitlab.base_url, "http://someone@gitlab.example.test");
        assert_eq!(s.poll_interval_secs, 60);
        assert_eq!(s.github.account.as_deref(), Some("work"));
        // Saving it as is fails with the reason, until the URL is fixed.
        assert!(s
            .clone()
            .validated()
            .unwrap_err()
            .to_string()
            .contains("https://"));
    }

    #[test]
    fn validation_and_interval_floor() {
        let mut s = Settings::default();
        s.gitlab.base_url = " https://gitlab.example.test/ ".into();
        s.github.account = Some("  ".into());
        s.poll_interval_secs = 1;
        let s = s.validated().unwrap();
        assert_eq!(s.gitlab.base_url, "https://gitlab.example.test");
        assert_eq!(s.github.account, None);
        assert_eq!(
            s.poll_interval(),
            Duration::from_secs(MIN_POLL_INTERVAL_SECS)
        );

        let mut bad = Settings::default();
        bad.gitlab.base_url = "gitlab.example.test".into();
        assert!(matches!(bad.validated(), Err(CoreError::InvalidInput(_))));
    }

    #[test]
    fn plain_http_is_allowed_for_any_gitlab_host() {
        for url in [
            "http://gitlab.vpn.test",
            "http://10.0.0.5:8080/",
            "http://localhost:8080",
            "http://127.0.0.1:3000/",
            "http://[::1]:8080",
        ] {
            let mut s = Settings::default();
            s.gitlab.base_url = url.into();
            assert!(s.validated().is_ok(), "{url}");
        }
        let mut s = Settings::default();
        s.gitlab.base_url = " http://GitLab.vpn.test:8080/ ".into();
        assert_eq!(
            s.validated().unwrap().gitlab.base_url,
            "http://gitlab.vpn.test:8080"
        );
        // Still refused: userinfo, query, fragment, other schemes.
        for bad in [
            "http://someone@gitlab.vpn.test",
            "http://gitlab.vpn.test/?a=b",
            "http://gitlab.vpn.test/#x",
            "ftp://gitlab.vpn.test",
            "gitlab.vpn.test",
        ] {
            let mut s = Settings::default();
            s.gitlab.base_url = bad.into();
            let err = s.validated().unwrap_err();
            assert!(err.to_string().contains("http://"), "{bad}: {err}");
        }
    }
}
