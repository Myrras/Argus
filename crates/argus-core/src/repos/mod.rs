//! The repositories Argus knows and their setup, for Settings →
//! Repositories: every repository of a stored review (any tab, closed ones
//! still stored included) plus every one the settings name (a clone, a
//! review skill, a review language). A repository seen for the first time
//! in a refresh is therefore listed at once, with nothing set up.
//!
//! Read-only: the setup itself is saved through
//! [`Core::update_settings`] (`clone_paths`, `review_skills`,
//! `review_languages`, all keyed `host/repo`). The clone check is a
//! filesystem look (the folder, its `.git`), a `git` call only for a folder
//! without one; nothing is fetched, checked out or written.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::provider::ProviderId;
use crate::settings::{ReviewLanguage, ReviewSkillChoice, Settings};
use crate::{git, Core, CoreError, CoreResult};

/// One repository and how it is set up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoSetup {
    /// `host/repo`, the key of every per-repository setting.
    pub key: String,
    pub host: String,
    /// `owner/name` (GitLab groups may nest deeper).
    pub repo: String,
    /// From its reviews, else told by the host the settings configure;
    /// `None` when neither says.
    pub provider: Option<ProviderId>,
    /// Its reviews in the active tabs (see [`crate::ReviewRecord::is_active`]).
    pub open_reviews: u32,
    /// The local clone as it is set; `None` when none is.
    pub clone_path: Option<PathBuf>,
    pub clone: CloneState,
    /// The review skill choice (`Auto` unless set).
    pub review_skill: ReviewSkillChoice,
    /// The repository's own review language; `None` follows the global
    /// default ([`crate::settings::AiSettings::review_language`]).
    pub review_language: Option<ReviewLanguage>,
}

/// What the local clone set for a repository is on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloneState {
    /// No clone is set.
    NotSet,
    /// The folder is there and is a git repository.
    Found,
    /// The folder is gone (or is no folder).
    Missing,
    /// The folder is there but is no git repository.
    NotRepository,
}

/// What `dir` is: found, missing or no repository. A `.git` entry is
/// enough (no process per repository for the list); a folder without one
/// may still be a repository git recognises (a bare one, say).
pub(crate) fn clone_state(dir: &Path) -> CloneState {
    if !dir.is_dir() {
        return CloneState::Missing;
    }
    if dir.join(".git").exists() || git::LocalClone::open(dir).is_some() {
        CloneState::Found
    } else {
        CloneState::NotRepository
    }
}

/// The provider of the user's account on `host` (`hosts`: the enabled
/// accounts), for a repository only the settings name.
fn provider_of_host(hosts: &[(ProviderId, String)], host: &str) -> Option<ProviderId> {
    hosts
        .iter()
        .find(|(_, h)| h.eq_ignore_ascii_case(host))
        .map(|(p, _)| *p)
}

/// `host/repo` split at its first slash.
fn split_key(key: &str) -> (String, String) {
    match key.split_once('/') {
        Some((host, repo)) => (host.to_string(), repo.to_string()),
        None => (key.to_string(), String::new()),
    }
}

/// What the list shows, from what is stored: `reviews` are
/// `(host, repo, provider, active)` per stored review. Sorted: repositories
/// with open reviews first, then by key (case-insensitive).
pub(crate) fn list(
    settings: &Settings,
    hosts: &[(ProviderId, String)],
    reviews: impl IntoIterator<Item = (String, String, ProviderId, bool)>,
    clone_state: impl Fn(&Path) -> CloneState,
) -> Vec<RepoSetup> {
    // key → (host, repo, provider, open reviews)
    let mut known: BTreeMap<String, (String, String, Option<ProviderId>, u32)> = BTreeMap::new();
    for (host, repo, provider, active) in reviews {
        let key = format!("{host}/{repo}");
        let entry = known
            .entry(key)
            .or_insert_with(|| (host, repo, Some(provider), 0));
        entry.3 += u32::from(active);
    }
    let configured = settings
        .clone_paths
        .keys()
        .chain(settings.review_skills.keys())
        .chain(settings.review_languages.keys());
    for key in configured {
        if !known.contains_key(key) {
            let (host, repo) = split_key(key);
            let provider = provider_of_host(hosts, &host);
            known.insert(key.clone(), (host, repo, provider, 0));
        }
    }
    let mut out: Vec<RepoSetup> = known
        .into_iter()
        .map(|(key, (host, repo, provider, open_reviews))| {
            let clone_path = settings.clone_paths.get(&key).cloned();
            let clone = clone_path
                .as_deref()
                .map_or(CloneState::NotSet, &clone_state);
            RepoSetup {
                review_skill: settings
                    .review_skills
                    .get(&key)
                    .cloned()
                    .unwrap_or_default(),
                review_language: settings.review_languages.get(&key).cloned(),
                key,
                host,
                repo,
                provider,
                open_reviews,
                clone_path,
                clone,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        (b.open_reviews > 0)
            .cmp(&(a.open_reviews > 0))
            .then_with(|| a.key.to_lowercase().cmp(&b.key.to_lowercase()))
            .then_with(|| a.key.cmp(&b.key))
    });
    out
}

impl Core {
    /// Every repository Argus knows with its setup (see the module docs).
    /// Blocking: one filesystem look per clone set.
    pub fn list_repositories(&self) -> CoreResult<Vec<RepoSetup>> {
        let settings = self.settings();
        // Every stored review, a host switched off included: its setup
        // stays the user's to change.
        let reviews = self.store.reviews()?.into_iter().map(|r| {
            let active = r.is_active();
            (r.review.host, r.review.repo, r.review.provider, active)
        });
        let hosts = crate::accounts::enabled_hosts(&settings, self.accounts.cached().as_ref());
        Ok(list(&settings, &hosts, reviews, clone_state))
    }

    /// Show the clone set for `repo` (`host/repo`) in the file manager.
    /// Only the folder the settings hold for it (the webview names the
    /// repository, never a path).
    pub fn reveal_repo_clone(&self, repo: &str) -> CoreResult<()> {
        let settings = self.settings();
        let dir = settings.clone_paths.get(repo.trim()).ok_or_else(|| {
            CoreError::InvalidInput("no local clone is set for this repository".into())
        })?;
        let dir = std::fs::canonicalize(dir)
            .ok()
            .filter(|d| d.is_dir())
            .ok_or_else(|| {
                CoreError::InvalidInput("the local clone's folder is not there".into())
            })?;
        self.opener().reveal(&dir)
    }
}

#[cfg(test)]
mod tests;
