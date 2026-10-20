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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

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
pub use provider::{GithubStatus, GitlabProblem, GitlabStatus, ProviderStatus};
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

/// How a core turns settings into providers ([`provider::from_settings`];
/// tests swap in their own).
type ProviderBuilder = Box<dyn Fn(&Settings) -> Vec<Arc<dyn ReviewProvider>> + Send + Sync>;

/// Where a core stores the GitLab token for an instance URL (the OS
/// keychain, [`provider::gitlab::gitlab_token_set`]; tests swap in their
/// own).
type TokenWriter = Box<dyn Fn(&str, &str) -> CoreResult<()> + Send + Sync>;

/// The file next to `settings.json` that says the one-time move of a
/// plain-http GitLab token to its own keychain entry ran (see
/// [`provider::gitlab::gitlab_migrate_plain_http_token`]).
const PLAIN_HTTP_TOKEN_MOVED: &str = "gitlab-http-token-moved";

/// Run `f` unless `marker` exists, then create it. A marker that cannot be
/// written is logged: `f` runs again on the next start, which only matters
/// if the GitLab URL went from https to http in between.
fn once(marker: &Path, f: impl FnOnce()) {
    if marker.exists() {
        return;
    }
    f();
    let written = marker
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(marker, "done\n"));
    if written.is_err() {
        tracing::warn!("could not record the one-time GitLab token move");
    }
}

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
    /// What a settings save builds its providers with.
    build_providers: ProviderBuilder,
    /// What [`set_gitlab_token`](Self::set_gitlab_token) stores the token with.
    write_gitlab_token: TokenWriter,
    poller: Mutex<Option<(poll::Poller, RefreshCallback)>>,
    /// Held by a settings save and a poll start from reading the providers
    /// to putting the loop in place (see [`update_settings`](Self::update_settings)).
    settings_update: Mutex<()>,
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
    /// Set by [`shutdown`](Self::shutdown): no poll loop starts afterwards.
    shut_down: AtomicBool,
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
        // Before any provider reads the token.
        once(
            &settings_path.with_file_name(PLAIN_HTTP_TOKEN_MOVED),
            || provider::gitlab::gitlab_migrate_plain_http_token(&settings.gitlab.base_url),
        );
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
            build_providers: Box::new(provider::from_settings),
            write_gitlab_token: Box::new(provider::gitlab::gitlab_token_set),
            poller: Mutex::new(None),
            settings_update: Mutex::new(()),
            content_keys: Mutex::new(HashMap::new()),
            commentable: Mutex::new(None),
            ai,
            shut_down: AtomicBool::new(false),
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
    ///
    /// Saves run one at a time, from writing the file to putting the new
    /// loop in place: two quick saves interleaved could otherwise store one
    /// save's settings and leave the loop on the other's providers (one
    /// reads the providers for its new loop, the other swaps them and finds
    /// no loop to restart, then the first puts its stale loop in place).
    pub fn update_settings(&self, settings: Settings) -> CoreResult<Settings> {
        let settings = settings.validated()?;
        let one_at_a_time = self
            .settings_update
            .lock()
            .expect("settings update lock poisoned");
        if let Some(path) = &self.settings_path {
            settings.save(path)?;
        }
        *self.settings.lock().expect("settings lock poisoned") = settings.clone();
        self.ai.set_enabled(settings.ai.enabled);
        self.ai.set_adapter(&settings.ai.adapter);
        self.rebuild_providers(&settings, one_at_a_time);
        Ok(settings)
    }

    /// Store the GitLab token for the configured instance in the OS keychain
    /// (a blank token deletes it). Like a settings save, the providers are
    /// rebuilt and a running poll loop restarts with them.
    ///
    /// The Settings form saves the URL first and the token right after: the
    /// loop the URL save restarted has already refreshed (or is refreshing)
    /// without a token, and would report "GitLab is not set up" until the
    /// next poll. Restarting drops that refresh's report and reads GitLab
    /// with the token at once. Fresh providers also forget what the old
    /// token's user and conditional cache held.
    pub fn set_gitlab_token(&self, token: &str) -> CoreResult<()> {
        let one_at_a_time = self
            .settings_update
            .lock()
            .expect("settings update lock poisoned");
        let settings = self.settings();
        if settings.gitlab.base_url.is_empty() {
            return Err(CoreError::InvalidInput(
                "set the GitLab URL before its token".into(),
            ));
        }
        (self.write_gitlab_token)(&settings.gitlab.base_url, token)?;
        self.rebuild_providers(&settings, one_at_a_time);
        Ok(())
    }

    /// Build the providers for `settings` and restart a running poll loop
    /// with them. `one_at_a_time` is the held `settings_update` lock: it is
    /// let go once the new loop is in place, before the old one's grace.
    fn rebuild_providers(&self, settings: &Settings, one_at_a_time: MutexGuard<'_, ()>) {
        *self.providers.lock().expect("providers lock poisoned") = (self.build_providers)(settings);
        let running = self.poller.lock().expect("poller lock poisoned").take();
        let Some((poller, callback)) = running else {
            return;
        };
        // The old loop is told to stop, not waited for: a refresh it has on
        // the network (up to a minute) would hold up the save. It ends on
        // its own and reports nothing; the new loop's first refresh queues
        // behind it on the refresh lock.
        poller.request_stop();
        self.start_polling_with(callback);
        // The next save may go ahead while this one gives the old loop its
        // grace.
        drop(one_at_a_time);
        if !poller.stop_within(POLL_STOP_GRACE) {
            tracing::debug!("a refresh was still under way when the settings changed");
        }
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

    /// Refresh every provider once (blocking). A refresh already under way
    /// (the poll's) is waited for first: refreshes run one at a time.
    pub fn refresh(&self) -> RefreshReport {
        poll::refresh_all_unless(&self.providers(), &self.store, &Utc::now, None)
            .expect("no stop flag to stop on")
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
    ///
    /// A left side the platform could only approximate (GitHub without a
    /// merge base for a large PR) is replaced by the local clone's merge
    /// base when one is configured and has both commits.
    pub fn list_files(&self, id: ReviewId, since_last_review: bool) -> CoreResult<ReviewFiles> {
        let record = self.review(id)?;
        let provider = self.provider_for(&record)?;
        let (mut diff, keys, own) = files::fetch(provider.as_ref(), &record, since_last_review)?;
        self.exact_base(&record, &mut diff);
        // The left side comments go on, resolved the same way
        // `commentable_lines` resolves it.
        let review_base_sha = match own {
            Some(mut own) => {
                self.exact_base(&record, &mut own);
                own.base_sha
            }
            None => diff.base_sha.clone(),
        };
        let marks = self.store.viewed(id)?;
        let out = files::assemble(id, diff, review_base_sha, &keys, &marks, since_last_review);
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

    /// [`exact_base`] with this core's settings.
    pub(crate) fn exact_base(&self, record: &ReviewRecord, diff: &mut provider::FileDiff) {
        exact_base(&self.settings(), record, diff);
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
        // Like a save: a save swapping the providers in between this start
        // reading them and putting its loop in place would leave it stale.
        let _one_at_a_time = self
            .settings_update
            .lock()
            .expect("settings update lock poisoned");
        self.start_polling_with(Arc::new(on_refresh));
    }

    /// Start a loop over the current providers. Callers hold
    /// `settings_update`.
    fn start_polling_with(&self, callback: RefreshCallback) {
        self.stop_polling();
        let providers = self.providers();
        #[cfg(test)]
        AFTER_PROVIDERS_READ.with(|h| h.borrow_mut().take().map(|f| f()));
        let interval = self.settings().poll_interval();
        let mut slot = self.poller.lock().expect("poller lock poisoned");
        // Checked under the lock `shutdown` sets it under: a start racing a
        // quit (a settings save restarting the loop) either is stopped by
        // it or does nothing.
        if self.shut_down.load(Ordering::SeqCst) {
            return;
        }
        let cb = callback.clone();
        let poller = poll::Poller::start(providers, self.store.clone(), interval, move |r| cb(r));
        let replaced = slot.replace((poller, callback));
        drop(slot);
        // Another start got in between: stop its loop, outside the lock.
        if let Some((old, _)) = replaced {
            old.stop();
        }
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
        let poller = {
            let mut slot = self.poller.lock().expect("poller lock poisoned");
            self.shut_down.store(true, Ordering::SeqCst);
            slot.take()
        };
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

#[cfg(test)]
thread_local! {
    /// Run once inside `start_polling_with` right after it read the
    /// providers for the new loop (tests only): where another settings save
    /// would come in.
    pub(crate) static AFTER_PROVIDERS_READ: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Replace a stand-in left side ([`provider::FileDiff::base_approximate`])
/// by the local clone's merge base, when a clone is configured and has both
/// commits. Every reader of a review's diff that shows, offers, numbers or
/// posts old-side lines (the code view, the commentable lines, posting, an
/// AI run) goes through it, so they agree on the left side: a comment read
/// against one and posted against the other would be refused.
///
/// The stand-in is the platform's base branch tip (T2). When the clone does
/// not have it (the base branch moved on since the clone was fetched), the
/// stand-in stays and the old side stays read-only until the user fetches
/// their clone. No other commit is used in its place: the base commit stored
/// with the review does not give the merge base reliably (a retargeted base,
/// a stacked PR merged into the base or a force-pushed base would mark a
/// wrong left side as exact).
pub(crate) fn exact_base(
    settings: &Settings,
    record: &ReviewRecord,
    diff: &mut provider::FileDiff,
) {
    if !diff.base_approximate {
        return;
    }
    let Some(clone) = settings
        .clone_path(&record.review)
        .and_then(git::LocalClone::open)
    else {
        return;
    };
    // No answer when the clone lacks either commit.
    if let Some(sha) = clone.merge_base(&diff.base_sha, &diff.head_sha) {
        diff.base_sha = sha;
        diff.base_approximate = false;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn the_plain_http_token_move_runs_once_ever() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join(PLAIN_HTTP_TOKEN_MOVED);
        let runs = Cell::new(0);
        once(&marker, || runs.set(runs.get() + 1));
        once(&marker, || runs.set(runs.get() + 1));
        assert_eq!(runs.get(), 1);
        assert!(marker.exists());
        // Opening a core leaves the marker next to its settings: a later
        // switch from https to http is never "moved" to plain http.
        let other = tempfile::tempdir().unwrap();
        Core::open(
            &other.path().join("argus.db"),
            &other.path().join("settings.json"),
        )
        .unwrap();
        assert!(other.path().join(PLAIN_HTTP_TOKEN_MOVED).exists());
    }
}
