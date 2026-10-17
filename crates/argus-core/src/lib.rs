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
//! - [`files`] — the code view's file list and viewed marks.
//! - [`post`] — posting comments, replies and approvals (explicit only).
//! - [`history`] — then/now thread history: line walk, snapshots, timeline.
//! - [`ai`] — the AI pass: headless agent runs, findings, checklist, verdicts.
//! - [`git`] — a configured local clone, read-only: history walks and file
//!   contents without the API, which stays the fallback.

#![forbid(unsafe_code)]

pub mod ai;
pub mod error;
pub mod files;
pub mod git;
pub mod history;
pub mod ids;
mod platform;
pub mod poll;
pub mod post;
pub mod provider;
pub mod review;
pub mod settings;
pub mod store;

#[cfg(test)]
mod testutil;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::Utc;

pub use ai::{
    AiRunState, AiRunStatus, AiStateCallback, ChecklistId, ChecklistItem, Finding, FindingId,
    FindingPost, FindingPostOutcome, FindingPostResult, Severity, Verdict, VerdictLabel,
};
pub use error::{CoreError, CoreResult};
pub use files::{FileEntry, ReviewFiles};
pub use history::{LineStatus, ThreadHistory};
pub use ids::{ReviewId, ThreadId};
pub use poll::{RefreshError, RefreshReport};
pub use post::{
    CommentableLines, NewComment, PostError, PostErrorKind, PostResult, PostedComment, PostedNote,
};
pub use provider::{GithubStatus, GitlabStatus, ProviderStatus};
pub use review::{ReviewStatus, RoundReason};
pub use settings::Settings;
pub use store::{ReviewRecord, StoredThread};

use provider::ReviewProvider;
use store::Store;

/// How long [`Core::shutdown`] waits for a refresh under way before leaving
/// it behind: short enough that quitting feels immediate.
pub const POLL_STOP_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// What [`Core::start_polling`] calls after every refresh.
pub type RefreshCallback = Arc<dyn Fn(&RefreshReport) + Send + Sync>;

/// The application's single entry point.
///
/// Cheap to share: hold it behind an `Arc` in the shell and clone the handle
/// into any detached task. Interior mutability ([`Mutex`]) keeps the public API
/// `&self`, so callers never need a mutable borrow.
///
/// Provider calls block (network, `gh`): call [`refresh`](Self::refresh),
/// [`file_at`](Self::file_at), [`list_files`](Self::list_files) and
/// [`set_file_viewed`](Self::set_file_viewed) from `spawn_blocking`, never
/// the UI thread.
pub struct Core {
    store: Arc<Store>,
    /// `None` for an in-memory core (tests): settings are not persisted.
    settings_path: Option<PathBuf>,
    settings: Mutex<Settings>,
    providers: Mutex<Vec<Arc<dyn ReviewProvider>>>,
    poller: Mutex<Option<(poll::Poller, RefreshCallback)>>,
    /// Per review: the head its content keys were read at, and the keys
    /// (path → content sha) — what a viewed mark is stored with. Memory
    /// only; refetched when the head moved.
    content_keys: Mutex<HashMap<ReviewId, (String, files::ContentKeys)>>,
    /// The commentable lines of the last review asked for, per file, at its
    /// stored head: the code view asks once per file, and every file would
    /// otherwise refetch the whole diff. Memory only.
    commentable: Mutex<Option<post::CommentableCache>>,
    /// The AI pass' scheduler (see [`ai`]).
    ai: ai::runtime::AiRuntime,
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
        let store = Arc::new(store);
        let ai = ai::runtime::AiRuntime::new(
            store.clone(),
            settings.ai.enabled,
            Arc::new(ai::job::execute),
            ai::MAX_CONCURRENT,
        );
        ai.set_adapter(&settings.ai.adapter);
        Self {
            store,
            settings_path,
            settings: Mutex::new(settings),
            providers: Mutex::new(providers),
            poller: Mutex::new(None),
            content_keys: Mutex::new(HashMap::new()),
            commentable: Mutex::new(None),
            ai,
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
        self.ai.set_enabled(settings.ai.enabled);
        self.ai.set_adapter(&settings.ai.adapter);
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
    ///
    /// Read, classified and written in one store transaction, and only the
    /// watermark and its classification are written: a poll storing at the
    /// same time can neither lose the watermark nor be undone by it.
    pub fn mark_reviewed(&self, id: ReviewId) -> CoreResult<ReviewRecord> {
        let now = Utc::now();
        self.store.transaction(|conn| {
            let record = store::read_review(conn, id)?.ok_or(CoreError::NotFound)?;
            let threads: Vec<_> = store::read_threads(conn, id)?
                .into_iter()
                .map(|t| t.thread)
                .collect();
            let next = review::mark_reviewed(&record, &threads, now);
            store::write_mark(conn, &next)?;
            Ok(next)
        })
    }

    /// The provider a review belongs to.
    fn provider_for(&self, record: &ReviewRecord) -> CoreResult<Arc<dyn ReviewProvider>> {
        self.providers()
            .into_iter()
            .find(|p| p.id() == record.review.provider && p.host() == record.review.host)
            .ok_or_else(|| CoreError::Provider(format!("{} is not set up", record.review.provider)))
    }

    /// The content of `path` at `sha` in the review's repository (blocking):
    /// from the local clone when one is configured and has the commit, from
    /// the provider otherwise.
    pub fn file_at(&self, id: ReviewId, sha: &str, path: &str) -> CoreResult<String> {
        let record = self.review(id)?;
        let settings = self.settings();
        let clone = settings
            .clone_path(&record.review)
            .and_then(git::LocalClone::open);
        git::file_or(clone.as_ref(), sha, path, || {
            self.provider_for(&record)?
                .file_at(&record.review, sha, path)
        })
    }

    /// The review's changed files with their viewed marks (blocking): its
    /// own diff, or with `since_last_review` the diff from the commit last
    /// marked reviewed to head (an error when there is none, or it is the
    /// head).
    pub fn list_files(&self, id: ReviewId, since_last_review: bool) -> CoreResult<ReviewFiles> {
        let record = self.review(id)?;
        let provider = self.provider_for(&record)?;
        let (diff, keys) = files::fetch(provider.as_ref(), &record, since_last_review)?;
        let marks = self.store.viewed(id)?;
        let out = files::assemble(id, diff, &keys, &marks, since_last_review);
        self.remember_keys(id, &record.head_sha, keys);
        Ok(out)
    }

    /// Mark a changed file viewed (at its content at head) or not; returns
    /// the review's viewed paths afterwards. Blocking: reads the review's
    /// diff when its content keys are not known for the current head.
    pub fn set_file_viewed(
        &self,
        id: ReviewId,
        path: &str,
        viewed: bool,
    ) -> CoreResult<Vec<String>> {
        let record = self.review(id)?;
        let keys = self.keys_for(&record)?;
        if viewed {
            let sha = keys.get(path).ok_or_else(|| {
                CoreError::InvalidInput("not a changed file of this review".into())
            })?;
            self.store.set_viewed(id, path, sha, Utc::now())?;
        } else {
            self.store.clear_viewed(id, path)?;
        }
        Ok(store::viewed_paths(&self.store.viewed(id)?, &keys))
    }

    fn remember_keys(&self, id: ReviewId, head: &str, keys: files::ContentKeys) {
        self.content_keys
            .lock()
            .expect("content keys lock poisoned")
            .insert(id, (head.to_string(), keys));
    }

    /// The review's content keys at its current head, fetched when unknown.
    fn keys_for(&self, record: &ReviewRecord) -> CoreResult<files::ContentKeys> {
        let cached = self
            .content_keys
            .lock()
            .expect("content keys lock poisoned")
            .get(&record.id)
            .filter(|(head, _)| *head == record.head_sha)
            .map(|(_, keys)| keys.clone());
        if let Some(keys) = cached {
            return Ok(keys);
        }
        let diff = self
            .provider_for(record)?
            .fetch_files(&record.review, &files::review_range(record))?;
        let keys = files::content_keys(&diff);
        self.remember_keys(record.id, &record.head_sha, keys.clone());
        Ok(keys)
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

    /// Stop everything that runs in the background, for quitting: the poll
    /// loop is told to stop, AI runs are cancelled and their agents killed
    /// (waiting up to [`ai::SHUTDOWN_GRACE`] for the runs to remove their
    /// temp dirs), then a refresh still under way gets at most
    /// [`POLL_STOP_GRACE`] before it is left behind: quitting never waits
    /// on the network. Nothing starts afterwards. Blocking.
    pub fn shutdown(&self) {
        let poller = self.poller.lock().expect("poller lock poisoned").take();
        if let Some((p, _)) = &poller {
            p.request_stop();
        }
        if !self.ai.shutdown(ai::SHUTDOWN_GRACE) {
            tracing::warn!("AI runs were still ending when Argus quit");
        }
        if let Some((p, _)) = poller {
            if !p.stop_within(POLL_STOP_GRACE) {
                tracing::debug!("a refresh was still under way when Argus quit");
            }
        }
    }

    /// Ask a running loop to refresh now; a no-op when not polling.
    pub fn poll_now(&self) {
        if let Some((p, _)) = self.poller.lock().expect("poller lock poisoned").as_ref() {
            p.refresh_now();
        }
    }
}
