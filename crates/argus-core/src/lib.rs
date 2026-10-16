//! argus-core — the UI-agnostic heart of Argus, a review cockpit for
//! GitLab merge requests and GitHub pull requests.
//!
//! Owns all domain logic and state behind a single entry point, [`Core`], and
//! has **zero** dependency on Tauri (or any UI). The Tauri shell (`src-tauri`)
//! is a thin adapter over [`Core`]'s public API, which means the whole domain is
//! testable on plain `cargo test` without spinning up a window.
//!
//! ## The core contract
//! All mutation flows through [`Core`]. Keep orchestration/domain logic here;
//! the shell should only (de)serialize arguments and forward calls. This keeps
//! the core compiling — and testable — without the UI.
//!
//! ## Modules
//! - [`provider`] — the GitHub (`gh` CLI) and GitLab (REST) clients behind
//!   one [`ReviewProvider`](provider::ReviewProvider) trait.
//! - [`store`] — SQLite review state.
//! - [`settings`] — `settings.json`.
//! - [`review`] — round detection, pure.
//! - [`poll`] — refresh and the background loop.
//!
//! The greet + notes sample (inherited from the boilerplate) keeps the
//! core → IPC → frontend wiring tested until the IPC rewrite replaces it.

#![forbid(unsafe_code)]

pub mod error;
pub mod ids;
pub mod poll;
pub mod provider;
pub mod review;
pub mod settings;
pub mod store;

#[cfg(test)]
mod testutil;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use serde::{Deserialize, Serialize};

pub use error::{CoreError, CoreResult};
pub use ids::{NoteId, ReviewId, ThreadId};
pub use poll::{RefreshError, RefreshReport};
pub use provider::{GithubStatus, GitlabStatus, ProviderStatus};
pub use review::{ReviewStatus, RoundReason};
pub use settings::Settings;
pub use store::{ReviewRecord, StoredThread};

use provider::ReviewProvider;
use store::Store;

/// What [`Core::start_polling`] calls after every refresh.
pub type RefreshCallback = Arc<dyn Fn(&RefreshReport) + Send + Sync>;

/// One note. Serializable so it can cross the IPC boundary unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Note {
    pub id: NoteId,
    pub text: String,
}

/// The application's single entry point.
///
/// Cheap to share: hold it behind an `Arc` in the shell and clone the handle
/// into any detached task. Interior mutability ([`Mutex`]) keeps the public API
/// `&self`, so callers never need a mutable borrow.
///
/// Provider calls block (network, `gh`): call [`refresh`](Self::refresh)
/// and [`file_at`](Self::file_at) from `spawn_blocking`, never the UI thread.
pub struct Core {
    notes: Mutex<Vec<Note>>,
    store: Arc<Store>,
    /// `None` for an in-memory core (tests): settings are not persisted.
    settings_path: Option<PathBuf>,
    settings: Mutex<Settings>,
    providers: Mutex<Vec<Arc<dyn ReviewProvider>>>,
    poller: Mutex<Option<(poll::Poller, RefreshCallback)>>,
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}

impl Core {
    /// An in-memory core with default settings: nothing is persisted.
    pub fn new() -> Self {
        let store = Store::in_memory().expect("open an in-memory database");
        Self::with_parts(store, None, Settings::default(), None)
    }

    /// The real app: the database at `db_path` and `settings.json` at
    /// `settings_path` (both created on first use).
    pub fn open(db_path: &Path, settings_path: &Path) -> CoreResult<Self> {
        let store = Store::open(db_path)?;
        let settings = Settings::load(settings_path);
        Ok(Self::with_parts(
            store,
            Some(settings_path.to_path_buf()),
            settings,
            None,
        ))
    }

    /// A core over explicit providers (tests, or a shell that builds its
    /// own). Settings stay in memory.
    pub fn with_providers(store: Store, providers: Vec<Arc<dyn ReviewProvider>>) -> Self {
        Self::with_parts(store, None, Settings::default(), Some(providers))
    }

    fn with_parts(
        store: Store,
        settings_path: Option<PathBuf>,
        settings: Settings,
        providers: Option<Vec<Arc<dyn ReviewProvider>>>,
    ) -> Self {
        let providers = providers.unwrap_or_else(|| provider::from_settings(&settings));
        Self {
            notes: Mutex::new(Vec::new()),
            store: Arc::new(store),
            settings_path,
            settings: Mutex::new(settings),
            providers: Mutex::new(providers),
            poller: Mutex::new(None),
        }
    }

    fn providers(&self) -> Vec<Arc<dyn ReviewProvider>> {
        self.providers
            .lock()
            .expect("providers lock poisoned")
            .clone()
    }

    pub fn settings(&self) -> Settings {
        self.settings
            .lock()
            .expect("settings lock poisoned")
            .clone()
    }

    /// Validate, persist and apply new settings; returns what was stored.
    /// The providers are rebuilt and a running poll loop restarts with them.
    pub fn update_settings(&self, settings: Settings) -> CoreResult<Settings> {
        let settings = settings.validated()?;
        if let Some(path) = &self.settings_path {
            settings.save(path)?;
        }
        *self.settings.lock().expect("settings lock poisoned") = settings.clone();
        *self.providers.lock().expect("providers lock poisoned") =
            provider::from_settings(&settings);
        let running = self.poller.lock().expect("poller lock poisoned").take();
        if let Some((poller, callback)) = running {
            poller.stop();
            self.start_polling_with(callback);
        }
        Ok(settings)
    }

    /// Store the GitLab token for the configured instance in the OS keychain
    /// (a blank token deletes it).
    pub fn set_gitlab_token(&self, token: &str) -> CoreResult<()> {
        let url = self.settings().gitlab.base_url;
        if url.is_empty() {
            return Err(CoreError::InvalidInput(
                "set the GitLab URL before its token".into(),
            ));
        }
        provider::gitlab::gitlab_token_set(&url, token)
    }

    /// Whether a GitLab token is stored for the configured instance.
    pub fn gitlab_token_present(&self) -> bool {
        let url = self.settings().gitlab.base_url;
        !url.is_empty() && provider::gitlab::gitlab_token_present(&url)
    }

    /// Whether each provider can be used right now: `gh` installed and
    /// logged in for the configured host (a local check, no network), and
    /// GitLab configured with a token. Blocking (runs `gh`).
    pub fn provider_status(&self) -> ProviderStatus {
        provider::status(&self.settings())
    }

    /// Every tracked review, most recently updated first. Filter on
    /// [`ReviewRecord::is_active`] and [`ReviewRecord::status`] for the tabs.
    pub fn reviews(&self) -> CoreResult<Vec<ReviewRecord>> {
        self.store.reviews()
    }

    pub fn review(&self, id: ReviewId) -> CoreResult<ReviewRecord> {
        self.store.review(id)?.ok_or(CoreError::NotFound)
    }

    /// The stored threads of a review (as of the last refresh).
    pub fn threads(&self, id: ReviewId) -> CoreResult<Vec<StoredThread>> {
        self.review(id)?;
        self.store.threads(id)
    }

    /// Refresh every provider once (blocking).
    pub fn refresh(&self) -> RefreshReport {
        poll::refresh_all(&self.providers(), &self.store, Utc::now())
    }

    /// The user finished this round: move the watermark to the current head.
    pub fn mark_reviewed(&self, id: ReviewId) -> CoreResult<ReviewRecord> {
        let record = self.review(id)?;
        let threads: Vec<_> = self
            .store
            .threads(id)?
            .into_iter()
            .map(|t| t.thread)
            .collect();
        let next = review::mark_reviewed(&record, &threads, Utc::now());
        self.store.upsert_review(&next)?;
        Ok(next)
    }

    /// The content of `path` at `sha` in the review's repository (blocking).
    pub fn file_at(&self, id: ReviewId, sha: &str, path: &str) -> CoreResult<String> {
        let record = self.review(id)?;
        let provider = self
            .providers()
            .into_iter()
            .find(|p| p.id() == record.review.provider && p.host() == record.review.host)
            .ok_or_else(|| {
                CoreError::Provider(format!("{} is not set up", record.review.provider))
            })?;
        provider.file_at(&record.review, sha, path)
    }

    /// Start the background refresh loop at the settings' interval (replacing
    /// a running one). `on_refresh` runs on the poll thread after every
    /// refresh.
    pub fn start_polling(&self, on_refresh: impl Fn(&RefreshReport) + Send + Sync + 'static) {
        self.start_polling_with(Arc::new(on_refresh));
    }

    fn start_polling_with(&self, callback: RefreshCallback) {
        self.stop_polling();
        let cb = callback.clone();
        let poller = poll::Poller::start(
            self.providers(),
            self.store.clone(),
            self.settings().poll_interval(),
            move |r| cb(r),
        );
        *self.poller.lock().expect("poller lock poisoned") = Some((poller, callback));
    }

    /// Stop the background loop (waits for an in-flight refresh).
    pub fn stop_polling(&self) {
        let running = self.poller.lock().expect("poller lock poisoned").take();
        if let Some((p, _)) = running {
            p.stop();
        }
    }

    /// Ask a running loop to refresh now; a no-op when not polling.
    pub fn poll_now(&self) {
        if let Some((p, _)) = self.poller.lock().expect("poller lock poisoned").as_ref() {
            p.refresh_now();
        }
    }

    /// A pure, fallible sample command: greet `name`, rejecting blank input.
    ///
    /// Demonstrates input validation returning [`CoreError::InvalidInput`],
    /// which the shell maps to a stable IPC error `kind`.
    pub fn greet(&self, name: &str) -> CoreResult<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::InvalidInput("name must not be empty".into()));
        }
        Ok(format!("Hello, {name}! Welcome to Argus."))
    }

    /// Add a note and return it.
    pub fn add_note(&self, text: &str) -> CoreResult<Note> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CoreError::InvalidInput(
                "note text must not be empty".into(),
            ));
        }
        let note = Note {
            id: NoteId::new(),
            text: text.to_owned(),
        };
        self.notes
            .lock()
            .expect("notes lock poisoned")
            .push(note.clone());
        Ok(note)
    }

    /// All notes, in insertion order.
    pub fn list_notes(&self) -> Vec<Note> {
        self.notes.lock().expect("notes lock poisoned").clone()
    }

    /// Remove a note by id, erroring if it does not exist.
    pub fn remove_note(&self, id: NoteId) -> CoreResult<()> {
        let mut notes = self.notes.lock().expect("notes lock poisoned");
        let before = notes.len();
        notes.retain(|n| n.id != id);
        if notes.len() == before {
            return Err(CoreError::NotFound);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greet_rejects_blank_names() {
        let core = Core::new();
        assert!(core.greet("   ").is_err());
        assert_eq!(core.greet("Ada").unwrap(), "Hello, Ada! Welcome to Argus.");
    }

    #[test]
    fn add_then_remove_note() {
        let core = Core::new();
        let note = core.add_note("first").unwrap();
        assert_eq!(core.list_notes().len(), 1);
        core.remove_note(note.id).unwrap();
        assert!(core.list_notes().is_empty());
        // Removing a missing id is an error, not a silent no-op.
        assert!(matches!(
            core.remove_note(note.id),
            Err(CoreError::NotFound)
        ));
    }
}
