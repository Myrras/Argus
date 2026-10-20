//! `settings.json`: hosts and accounts, polling, AI, local clones, the
//! review skill per repository.
//!
//! Load is forgiving (a missing or corrupt file is the defaults, a missing
//! field is its default) and save is atomic (temp file + rename), as in
//! Podium. No secrets live here: GitHub's token stays with `gh`, GitLab's in
//! the OS keychain.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::provider::gitlab::normalize_base_url;
use crate::provider::ReviewRef;

pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 180;

/// The one-time move of a plain-http GitLab token to its own keychain entry
/// (see [`crate::provider::gitlab::gitlab_migrate_plain_http_token`]).
pub(crate) const GITLAB_HTTP_TOKEN: &str = "gitlabHttpToken";
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
    /// Per-extension editor (Iris's `editorOverrides`): lower-case
    /// extension without the dot (`"md"`) → the application to open a
    /// review's file from the local clone with. See [`crate::open`].
    pub editor_overrides: BTreeMap<String, String>,
    /// The application for every other file; `None` is the OS default.
    pub default_editor: Option<String>,
    /// The code-review skill the AI pass follows, per repository (keyed
    /// like [`clone_paths`](Self::clone_paths)). A repository without an
    /// entry is [`ReviewSkillChoice::Auto`]; an older settings file has
    /// none, so it needs no migration step.
    pub review_skills: BTreeMap<String, ReviewSkillChoice>,
}

/// Where a review skill comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillSource {
    /// `~/.claude/skills/<dir>/SKILL.md`.
    User,
    /// An installed Claude Code plugin's `skills/<dir>/SKILL.md`, named
    /// `plugin:skill`.
    Plugin,
    /// The repository's own `.claude/skills/<dir>/SKILL.md`, read at the
    /// review's target branch.
    Project,
}

impl SkillSource {
    pub fn as_str(self) -> &'static str {
        match self {
            SkillSource::User => "user",
            SkillSource::Plugin => "plugin",
            SkillSource::Project => "project",
        }
    }
}

/// Which code-review skill the AI pass of a repository's reviews follows.
/// On the wire `{"kind":"auto"}`, `{"kind":"none"}` or
/// `{"kind":"skill","source":"user","name":"code-review"}`. Read leniently:
/// an entry Argus cannot read (hand-edited, from a newer Argus) is `Auto`,
/// never a reason to drop the whole settings file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ReviewSkillChoice {
    /// The repository's own review skill when it has one, else none.
    #[default]
    Auto,
    /// No skill: Argus' own review instructions only.
    None,
    /// This skill; when it is gone at run time, `Auto` (the run says so).
    Skill { source: SkillSource, name: String },
}

impl<'de> Deserialize<'de> for ReviewSkillChoice {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "camelCase")]
        enum Strict {
            Auto,
            None,
            Skill { source: SkillSource, name: String },
        }
        let value = serde_json::Value::deserialize(d)?;
        Ok(match serde_json::from_value::<Strict>(value) {
            Ok(Strict::None) => ReviewSkillChoice::None,
            Ok(Strict::Skill { source, name }) if !name.trim().is_empty() => {
                ReviewSkillChoice::Skill { source, name }
            }
            _ => ReviewSkillChoice::Auto,
        })
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            github: GithubSettings::default(),
            gitlab: GitlabSettings::default(),
            poll_interval_secs: DEFAULT_POLL_INTERVAL_SECS,
            ai: AiSettings::default(),
            clone_paths: BTreeMap::new(),
            editor_overrides: BTreeMap::new(),
            default_editor: None,
            review_skills: BTreeMap::new(),
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

/// What `settings.json` holds: the settings, and the one-time steps the
/// core has run (`migrations`, by name). Those are the core's own
/// bookkeeping: never in [`Settings`], so they never go to the UI and a
/// save of what the UI sends cannot drop them.
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsFile {
    #[serde(flatten)]
    settings: Settings,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    migrations: Vec<String>,
}

impl SettingsFile {
    /// Read `path`: `None` for a missing file, an error for one that exists
    /// but cannot be read or parsed (what a caller that writes must not
    /// paper over with the defaults).
    fn read_checked(path: &Path) -> CoreResult<Option<Self>> {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        serde_json::from_str(&raw)
            .map(Some)
            .map_err(|e| CoreError::InvalidInput(format!("settings file is not valid JSON: {e}")))
    }

    /// Read `path`; a missing or corrupt file is the defaults.
    fn read(path: &Path) -> Self {
        Self::read_checked(path)
            .unwrap_or_else(|e| {
                tracing::warn!("settings file is unreadable, using defaults: {e}");
                None
            })
            .unwrap_or_default()
    }

    /// Atomically write to `path`: a temp file renamed over the old one, so
    /// a crash mid-write never leaves a torn file.
    fn write(&self, path: &Path) -> CoreResult<()> {
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
}

#[cfg(test)]
/// Whether `settings.json` at `path` records the one-time step `name`.
pub(crate) fn migration_done(path: &Path, name: &str) -> bool {
    SettingsFile::read(path)
        .migrations
        .iter()
        .any(|m| m == name)
}

/// Like [`migration_done`], but an error when `path` exists and cannot be
/// read or parsed: whether the step ran is then unknown, and recording it
/// would overwrite the file.
pub(crate) fn migration_status(path: &Path, name: &str) -> CoreResult<bool> {
    Ok(SettingsFile::read_checked(path)?.is_some_and(|f| f.migrations.iter().any(|m| m == name)))
}

/// Record the one-time step `name` in `settings.json` at `path` (saved
/// atomically, the settings in it unchanged). A file that exists but cannot
/// be read or parsed is left untouched and is an error.
pub(crate) fn record_migration(path: &Path, name: &str) -> CoreResult<()> {
    let mut file = SettingsFile::read_checked(path)?.unwrap_or_default();
    if file.migrations.iter().any(|m| m == name) {
        return Ok(());
    }
    file.migrations.push(name.to_string());
    file.write(path)
}

impl Settings {
    /// Read `path`; a missing or corrupt file is the defaults.
    pub fn load(path: &Path) -> Self {
        SettingsFile::read(path).settings
    }

    /// Atomically write to `path` (see [`SettingsFile::write`]), keeping
    /// the one-time steps the file records.
    pub fn save(&self, path: &Path) -> CoreResult<()> {
        SettingsFile {
            settings: self.clone(),
            migrations: SettingsFile::read(path).migrations,
        }
        .write(path)
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
        // As Iris tidies them: extensions trimmed, lower-cased and without
        // a leading dot; rows with a blank extension or app dropped.
        self.editor_overrides = std::mem::take(&mut self.editor_overrides)
            .into_iter()
            .map(|(ext, app)| {
                let ext = ext.trim().trim_start_matches('.').to_lowercase();
                (ext, app.trim().to_string())
            })
            .filter(|(ext, app)| !ext.is_empty() && !app.is_empty())
            .collect();
        self.default_editor = self
            .default_editor
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty());
        // Only what differs from the default is kept: keys trimmed like a
        // clone's, skill names trimmed, `Auto` (and a blank name) dropped.
        self.review_skills = std::mem::take(&mut self.review_skills)
            .into_iter()
            .filter_map(|(repo, choice)| {
                let repo = repo.trim().to_string();
                let choice = match choice {
                    ReviewSkillChoice::Skill { source, name } => {
                        let name = name.trim().to_string();
                        if name.is_empty() {
                            return None;
                        }
                        ReviewSkillChoice::Skill { source, name }
                    }
                    other => other,
                };
                (!repo.is_empty() && choice != ReviewSkillChoice::Auto).then_some((repo, choice))
            })
            .collect();
        Ok(self)
    }

    pub fn gitlab_enabled(&self) -> bool {
        !self.gitlab.base_url.trim().is_empty()
    }

    /// The poll interval, never below [`MIN_POLL_INTERVAL_SECS`].
    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_interval_secs.max(MIN_POLL_INTERVAL_SECS))
    }

    /// The review skill choice for `review`'s repository (`Auto` unless set).
    pub fn review_skill(&self, review: &ReviewRef) -> ReviewSkillChoice {
        self.review_skills
            .get(&repo_key(review))
            .cloned()
            .unwrap_or_default()
    }

    /// The local clone for `review`'s repository, if one is set.
    pub fn clone_path(&self, review: &ReviewRef) -> Option<&Path> {
        self.clone_paths
            .get(&repo_key(review))
            .map(PathBuf::as_path)
    }
}

/// The key of per-repository settings: `host/repo`, as the provider reports
/// them.
pub fn repo_key(review: &ReviewRef) -> String {
    format!("{}/{}", review.host, review.repo)
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
    fn one_time_steps_are_recorded_in_the_file_and_kept_by_every_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert!(!migration_done(&path, GITLAB_HTTP_TOKEN));
        // Recorded without a settings file yet: written with the defaults.
        record_migration(&path, GITLAB_HTTP_TOKEN).unwrap();
        assert!(migration_done(&path, GITLAB_HTTP_TOKEN));
        assert_eq!(Settings::load(&path), Settings::default());
        record_migration(&path, GITLAB_HTTP_TOKEN).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(raw.matches(GITLAB_HTTP_TOKEN).count(), 1, "{raw}");
        // A settings save (what the UI sends has no such field) keeps it.
        let s = Settings {
            poll_interval_secs: 60,
            ..Settings::default()
        };
        s.save(&path).unwrap();
        assert!(migration_done(&path, GITLAB_HTTP_TOKEN));
        assert_eq!(Settings::load(&path), s);
        // Never part of the settings the UI reads.
        let wire = serde_json::to_string(&Settings::load(&path)).unwrap();
        assert!(!wire.contains("migrations"), "{wire}");
        // A file from before has none.
        std::fs::write(&path, r#"{"pollIntervalSecs":60}"#).unwrap();
        assert!(!migration_done(&path, GITLAB_HTTP_TOKEN));
    }

    #[test]
    fn recording_a_step_never_overwrites_a_file_that_cannot_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        // Missing is fine: nothing to lose, nothing recorded yet.
        assert!(!migration_status(&path, GITLAB_HTTP_TOKEN).unwrap());
        let broken = "{\"pollIntervalSecs\": 60,}";
        std::fs::write(&path, broken).unwrap();
        assert!(migration_status(&path, GITLAB_HTTP_TOKEN).is_err());
        assert!(record_migration(&path, GITLAB_HTTP_TOKEN).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        // A directory in its place is unreadable, not missing.
        let dir_path = dir.path().join("as-dir.json");
        std::fs::create_dir(&dir_path).unwrap();
        assert!(migration_status(&dir_path, GITLAB_HTTP_TOKEN).is_err());
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

    fn widgets(number: u64) -> ReviewRef {
        ReviewRef {
            provider: ProviderId::Github,
            host: "github.com".into(),
            repo: "acme-fixture/widgets".into(),
            number,
            url: String::new(),
            title: String::new(),
            author: String::new(),
            updated_at: None,
        }
    }

    #[test]
    fn review_skills_round_trip_per_repository() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut s = Settings::default();
        s.review_skills.insert(
            "github.com/acme-fixture/widgets".into(),
            ReviewSkillChoice::Skill {
                source: SkillSource::Plugin,
                name: "fixture-tools:code-review".into(),
            },
        );
        s.review_skills.insert(
            "gitlab.example.test/acme-fixture/api".into(),
            ReviewSkillChoice::None,
        );
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            raw["reviewSkills"]["github.com/acme-fixture/widgets"],
            serde_json::json!({"kind": "skill", "source": "plugin", "name": "fixture-tools:code-review"})
        );
        assert_eq!(
            raw["reviewSkills"]["gitlab.example.test/acme-fixture/api"],
            serde_json::json!({"kind": "none"})
        );
        assert_eq!(
            s.review_skill(&widgets(1)),
            ReviewSkillChoice::Skill {
                source: SkillSource::Plugin,
                name: "fixture-tools:code-review".into()
            }
        );
        let mut other = widgets(1);
        other.repo = "acme-fixture/gadgets".into();
        assert_eq!(s.review_skill(&other), ReviewSkillChoice::Auto);
    }

    #[test]
    fn older_or_garbled_review_skills_load_as_auto() {
        // A settings.json from before review skills: every repository auto.
        let old: Settings = serde_json::from_str(r#"{"pollIntervalSecs": 60}"#).unwrap();
        assert!(old.review_skills.is_empty());
        assert_eq!(old.review_skill(&widgets(1)), ReviewSkillChoice::Auto);
        // A hand-edited or future entry never costs the other settings.
        let s: Settings = serde_json::from_str(
            r#"{"pollIntervalSecs": 60, "reviewSkills": {
                "github.com/acme-fixture/widgets": {"kind": "fromTheFuture"},
                "github.com/acme-fixture/a": {"kind": "skill", "source": "user"},
                "github.com/acme-fixture/b": "code-review",
                "github.com/acme-fixture/c": {"kind": "skill", "source": "user", "name": "  "},
                "github.com/acme-fixture/d": {"kind": "none"}
            }}"#,
        )
        .unwrap();
        assert_eq!(s.poll_interval_secs, 60);
        for repo in ["widgets", "a", "b", "c"] {
            assert_eq!(
                s.review_skills[&format!("github.com/acme-fixture/{repo}")],
                ReviewSkillChoice::Auto,
                "{repo}"
            );
        }
        assert_eq!(
            s.review_skills["github.com/acme-fixture/d"],
            ReviewSkillChoice::None
        );
    }

    #[test]
    fn validation_keeps_only_review_skill_choices_that_differ_from_auto() {
        let mut s = Settings::default();
        s.review_skills.insert(
            " github.com/acme-fixture/widgets ".into(),
            ReviewSkillChoice::None,
        );
        s.review_skills
            .insert("github.com/acme-fixture/a".into(), ReviewSkillChoice::Auto);
        s.review_skills.insert(
            "github.com/acme-fixture/b".into(),
            ReviewSkillChoice::Skill {
                source: SkillSource::User,
                name: " code-review ".into(),
            },
        );
        let s = s.validated().unwrap();
        assert_eq!(
            s.review_skills.into_iter().collect::<Vec<_>>(),
            [
                (
                    "github.com/acme-fixture/b".to_string(),
                    ReviewSkillChoice::Skill {
                        source: SkillSource::User,
                        name: "code-review".into()
                    }
                ),
                (
                    "github.com/acme-fixture/widgets".to_string(),
                    ReviewSkillChoice::None
                ),
            ]
        );
    }

    #[test]
    fn editor_settings_are_tidied_like_iris() {
        let mut s = Settings::default();
        assert!(s.editor_overrides.is_empty() && s.default_editor.is_none());
        s.editor_overrides = [
            ("  .MD ", "  /Applications/Typora.app  "),
            ("txt", "   "),
            ("  ", "/usr/bin/vim"),
        ]
        .into_iter()
        .map(|(e, a)| (e.to_string(), a.to_string()))
        .collect();
        s.default_editor = Some("   ".into());
        let s = s.validated().unwrap();
        assert_eq!(
            s.editor_overrides.into_iter().collect::<Vec<_>>(),
            [("md".to_string(), "/Applications/Typora.app".to_string())]
        );
        assert_eq!(s.default_editor, None);
        // An older settings.json without them loads with none set.
        let old: Settings = serde_json::from_str(r#"{"pollIntervalSecs": 60}"#).unwrap();
        assert!(old.editor_overrides.is_empty() && old.default_editor.is_none());
    }
}
