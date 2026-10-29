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
//! - [`accounts`] — the hosts `gh` and `glab` are logged in to, and which
//!   of them Argus polls (Settings → Accounts).
//! - [`provider`] — the GitHub (`gh` CLI) and GitLab (`glab` CLI) clients behind
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
//! - [`open`] — a review's file from the local clone in the user's editor.
//! - [`repos`] — the repositories Argus knows and their setup (clone,
//!   review skill, review language), for Settings → Repositories.
//! - [`scripts`] — the user's own submit scripts: run after a successful
//!   Submit review (only the ones ticked), and written with the AI's help.

#![forbid(unsafe_code)]

pub mod accounts;
pub mod ai;
pub mod cache;
pub mod error;
pub mod files;
pub mod git;
pub mod history;
pub mod ids;
pub mod open;
mod platform;
pub mod poll;
pub mod post;
pub mod provider;
pub mod repos;
pub mod review;
pub mod scripts;
pub mod settings;
pub mod store;

#[cfg(test)]
mod testutil;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::Utc;

pub use accounts::{Account, AccountState, ProviderSpec};
pub use ai::{
    AiRunState, AiRunStatus, AiStateCallback, ChecklistId, ChecklistItem, Finding, FindingId,
    FindingPost, FindingPostOutcome, FindingPostResult, RepeatOf, RepeatReason, ReviewSkill,
    ReviewSkillList, RunContext, Severity, UsedSkill, Verdict, VerdictLabel,
};
pub use error::{CoreError, CoreResult};
pub use files::{FileEntry, ReviewFiles};
pub use history::{LineStatus, ThreadHistory};
pub use ids::{ReviewId, ThreadId};
pub use open::{CloneStatus, CloneSuggestion, LocalFile, Opener};
pub use poll::{RefreshError, RefreshReport, ReviewRefresh};
pub use post::{
    CommentableLines, NewComment, OutsideDiff, PostError, PostErrorKind, PostResult, PostedComment,
    PostedNote,
};
pub use provider::{MyReviewState, ReviewVerdict};
pub use repos::{CloneState, RepoSetup};
pub use review::{ReviewStatus, RoundReason};
pub use scripts::{
    DraftStage, DraftTurn, GeneratedScript, ScriptDraft, ScriptDraftCallback, ScriptDraftProgress,
    ScriptDraftProgressCallback, ScriptDraftProgressEvent, ScriptDraftStatus,
    ScriptGenerationStatus, ScriptResult, ScriptStatus, ScriptsDone, ScriptsDoneCallback,
    SubmitScript,
};
pub use settings::{AccountChoice, ReviewLanguage, ReviewSkillChoice, Settings, SkillSource};
pub use store::{ReviewRecord, StoredThread};

use provider::ReviewProvider;
use store::Store;

/// How many provider reads [`Core::files_at`] runs at once: enough to hide
/// a round trip each, few enough to stay polite to the platform.
pub const FILE_READS: usize = 6;

/// How long [`Core::shutdown`] waits for a refresh under way before leaving
/// it behind: short enough that quitting feels immediate.
pub const POLL_STOP_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// What [`Core::start_polling`] calls after every refresh.
pub type RefreshCallback = Arc<dyn Fn(&RefreshReport) + Send + Sync>;

/// How a core turns one host into its provider ([`provider::build`] over
/// the CLIs; a core over explicit providers picks from those; tests swap in
/// their own). `None` builds nothing for it.
type ProviderBuilder = Box<dyn Fn(&ProviderSpec) -> Option<Arc<dyn ReviewProvider>> + Send + Sync>;

/// A provider and the spec it was built from: a rebuild keeps the instance
/// (its rate-limit pause, conditional cache and user) while the spec stays.
#[derive(Clone)]
struct Built {
    spec: ProviderSpec,
    provider: Arc<dyn ReviewProvider>,
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
    /// One per enabled host with a login ([`accounts::provider_specs`]).
    providers: Mutex<Vec<Built>>,
    /// What a new host's provider is built with.
    build_providers: ProviderBuilder,
    /// What gh and glab are logged in to, asked only when it matters.
    accounts: Arc<accounts::AccountBook>,
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
    /// Opens a clone's file in an editor ([`open`]); the shell sets it.
    opener: Mutex<Arc<dyn open::Opener>>,
    /// Claude Code's config directory, where the user's and plugins' review
    /// skills are ([`ai::skills`]). The real app's is `~/.claude` (or
    /// `CLAUDE_CONFIG_DIR`); an in-memory core has none.
    claude_dir: Mutex<Option<PathBuf>>,
    /// The submit scripts' queue (see [`scripts`]).
    scripts: scripts::runtime::ScriptRuntime,
    /// Where a script written with the AI's help is saved: `scripts/` next
    /// to `settings.json`; an in-memory core has none.
    scripts_dir: Mutex<Option<PathBuf>>,
    /// What "Write with AI" asks the agent with (tests swap in a fake).
    generator: Mutex<scripts::generate::Generator>,
    /// "Write with AI" conversations and their background runs.
    drafts: scripts::drafts::DraftRuntime,
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}

impl Core {
    /// An in-memory core with default settings and no providers: nothing
    /// is persisted, no CLI is asked.
    pub fn new() -> Self {
        let store = Store::in_memory().expect("open an in-memory database");
        Self::with_parts(store, None, Settings::default(), Some(Vec::new()))
    }

    /// The real app: the database at `db_path` and `settings.json` at
    /// `settings_path` (both created on first use). The providers follow
    /// the user's `gh` and `glab` logins, asked on the first
    /// [`list_accounts`](Self::list_accounts) (or provider use): never
    /// here, as this may run on the UI thread.
    pub fn open(db_path: &Path, settings_path: &Path) -> CoreResult<Self> {
        let store = Store::open(db_path)?;
        let settings = Settings::load(settings_path);
        let core = Self::with_parts(store, Some(settings_path.to_path_buf()), settings, None);
        core.set_claude_dir(ai::skills::claude_config_dir());
        core.set_submit_scripts_dir(
            settings_path
                .parent()
                .map(|dir| dir.join(scripts::SCRIPTS_DIR)),
        );
        Ok(core)
    }

    /// A core over explicit providers (tests, or a shell that builds its
    /// own): they are its accounts, each logged in, on unless the settings
    /// switch its host off. Settings stay in memory; no CLI is asked.
    pub fn with_providers(store: Store, providers: Vec<Arc<dyn ReviewProvider>>) -> Self {
        Self::with_parts(store, None, Settings::default(), Some(providers))
    }

    /// [`with_providers`](Self::with_providers) with `settings` (kept in
    /// memory) in place of the defaults.
    pub fn with_providers_and_settings(
        store: Store,
        settings: Settings,
        providers: Vec<Arc<dyn ReviewProvider>>,
    ) -> Self {
        Self::with_parts(store, None, settings, Some(providers))
    }

    /// A core whose accounts and providers come from the `gh` and `glab`
    /// programs given (tests: fake CLIs), as [`open`](Self::open)'s come
    /// from the installed ones. Settings stay in memory.
    pub fn with_cli(
        store: Store,
        settings: Settings,
        gh: impl Into<String>,
        glab: impl Into<String>,
    ) -> Self {
        let core = Self::with_parts(store, None, settings, None);
        core.accounts.set_source(accounts::Source::Cli {
            gh: Some(gh.into()),
            glab: Some(glab.into()),
        });
        core
    }

    /// `providers` `Some` makes a core over those (see
    /// [`with_providers`](Self::with_providers)); `None` asks the CLIs.
    fn with_parts(
        store: Store,
        settings_path: Option<PathBuf>,
        settings: Settings,
        providers: Option<Vec<Arc<dyn ReviewProvider>>>,
    ) -> Self {
        let (book, build_providers): (Arc<accounts::AccountBook>, ProviderBuilder) = match providers
        {
            Some(pool) => {
                let mut found = accounts::Discovered::empty();
                for p in &pool {
                    let login = accounts::HostLogin {
                        provider: p.id(),
                        host: p.host().to_string(),
                        state: AccountState::LoggedIn,
                        user: None,
                        logins: Vec::new(),
                        scheme: None,
                    };
                    if let accounts::CliReport::Hosts(hosts) = match p.id() {
                        provider::ProviderId::Github => &mut found.github,
                        provider::ProviderId::Gitlab => &mut found.gitlab,
                    } {
                        hosts.push(login);
                    }
                }
                let book = Arc::new(accounts::AccountBook::new(accounts::Source::Fixed(found)));
                let builder: ProviderBuilder = Box::new(move |spec: &ProviderSpec| {
                    pool.iter()
                        .find(|p| p.id() == spec.provider && p.host() == spec.host)
                        .cloned()
                });
                (book, builder)
            }
            None => {
                let book = Arc::new(accounts::AccountBook::new(accounts::Source::Cli {
                    gh: None,
                    glab: None,
                }));
                let programs = book.clone();
                let builder: ProviderBuilder = Box::new(move |spec: &ProviderSpec| {
                    let (gh, glab) = programs.programs()?;
                    provider::build(spec, (&gh, &glab))
                });
                (book, builder)
            }
        };
        // A fixed set is known at once; the CLIs' are asked later.
        let built: Vec<Built> = match book.cached() {
            Some(found) => accounts::provider_specs(&settings, &found)
                .into_iter()
                .filter_map(|spec| build_providers(&spec).map(|provider| Built { spec, provider }))
                .collect(),
            None => Vec::new(),
        };
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
            providers: Mutex::new(built),
            build_providers,
            accounts: book,
            poller: Mutex::new(None),
            settings_update: Mutex::new(()),
            content_keys: Mutex::new(HashMap::new()),
            commentable: Mutex::new(None),
            ai,
            shut_down: AtomicBool::new(false),
            opener: Mutex::new(Arc::new(open::NoOpener)),
            claude_dir: Mutex::new(None),
            scripts: scripts::runtime::ScriptRuntime::new(Arc::new(scripts::run::execute)),
            scripts_dir: Mutex::new(None),
            generator: Mutex::new(Arc::new(scripts::generate::spawn_agent)),
            drafts: scripts::drafts::DraftRuntime::new(),
        }
    }

    /// Where Claude Code's user config lives (the review skills of the user
    /// and of installed plugins); `None` looks for none. [`Core::open`] sets
    /// the real one; tests point it at a fixture.
    pub fn set_claude_dir(&self, dir: Option<PathBuf>) {
        *self.claude_dir.lock().expect("claude dir lock poisoned") = dir;
    }

    pub(crate) fn claude_dir(&self) -> Option<PathBuf> {
        self.claude_dir
            .lock()
            .expect("claude dir lock poisoned")
            .clone()
    }

    /// The review skills the picker offers for `repo` (`host/repo`, keyed
    /// like [`Settings::clone_paths`]): every skill of the user and of
    /// installed plugins whose name says "review", and the repository's own
    /// from its clone's default branch (`origin/HEAD`, else `HEAD`), when a
    /// clone is set. Read-only.
    pub fn list_review_skills(&self, repo: Option<&str>) -> CoreResult<ai::ReviewSkillList> {
        let settings = self.settings();
        let clone = repo
            .and_then(|r| settings.clone_paths.get(r.trim()))
            .and_then(|p| git::LocalClone::open(p));
        let head = clone.as_ref().and_then(|c| c.default_branch_head());
        let base = head
            .as_ref()
            .map(|sha| ai::context::BaseFiles::new(sha.clone(), clone.as_ref(), None));
        Ok(ai::skills::list(
            self.claude_dir().as_deref(),
            base.as_ref(),
        ))
    }

    /// Remove the review worktrees an earlier Argus left in the configured
    /// clones (it quit or crashed mid-run): only the ones it marked, never
    /// one a run of this process uses. How many went. For startup (the shell
    /// runs it off the main thread); each run also sweeps its clone first.
    pub fn sweep_review_worktrees(&self) -> usize {
        self.settings()
            .clone_paths
            .values()
            .filter_map(|p| git::LocalClone::open(p))
            .map(|c| c.sweep_review_worktrees())
            .sum()
    }

    /// The current providers, as they are (no CLI is asked).
    fn providers(&self) -> Vec<Arc<dyn ReviewProvider>> {
        self.providers
            .lock()
            .expect("providers lock poisoned")
            .iter()
            .map(|b| b.provider.clone())
            .collect()
    }

    /// The providers to call: the CLIs are asked first when they never
    /// were (a call before the startup check finished). Blocking.
    fn live_providers(&self) -> Vec<Arc<dyn ReviewProvider>> {
        if self.accounts.cached().is_none() {
            self.list_accounts(false);
        }
        self.providers()
    }

    fn specs(&self) -> Vec<ProviderSpec> {
        self.providers
            .lock()
            .expect("providers lock poisoned")
            .iter()
            .map(|b| b.spec.clone())
            .collect()
    }

    /// The user's hosts (Settings → Accounts): every host `gh` and `glab`
    /// are logged in to, with its login state and the user's choice, plus
    /// the hosts the settings name that they are not. The CLIs (`gh auth
    /// status`, `glab auth status`: a request per host) are asked the first
    /// time, when `recheck` (Check again), after a settings change of the
    /// accounts and after a provider call refused for the login or without
    /// its CLI; otherwise the last answer stands, kept current by the
    /// refreshes. When the answer changes which hosts have a provider, the
    /// providers follow and a running poll loop restarts. Blocking.
    pub fn list_accounts(&self, recheck: bool) -> Vec<Account> {
        let found = self.accounts.get(recheck);
        if accounts::provider_specs(&self.settings(), &found) != self.specs() {
            let one_at_a_time = self
                .settings_update
                .lock()
                .expect("settings update lock poisoned");
            // Again under the lock: a save in between may have done it.
            let specs = accounts::provider_specs(&self.settings(), &found);
            if specs != self.specs() {
                self.rebuild_providers(specs, one_at_a_time);
            }
        }
        accounts::list(&self.settings(), &found)
    }

    /// What a refresh of `polled` (provider, host) says about the logins:
    /// a host refused for its login, or without its CLI, has the CLIs asked
    /// again; a host whose calls went through is logged in.
    fn note_refresh(
        book: &accounts::AccountBook,
        polled: &[(provider::ProviderId, String)],
        report: &RefreshReport,
    ) {
        for (id, host) in polled {
            let mut errors = report
                .errors
                .iter()
                .filter(|e| e.provider == *id && e.host == *host && e.review.is_none())
                .peekable();
            if errors.peek().is_none() {
                book.note_reachable(*id, host);
            } else if errors.any(|e| e.unavailable) {
                book.mark_stale();
            }
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings
            .lock()
            .expect("settings lock poisoned")
            .clone()
    }

    /// Validate, persist and apply new settings; returns what was stored.
    /// The providers follow a change of the accounts and a running poll
    /// loop restarts.
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
        let before = std::mem::replace(
            &mut *self.settings.lock().expect("settings lock poisoned"),
            settings.clone(),
        );
        self.ai.set_enabled(settings.ai.enabled);
        self.ai.set_adapter(&settings.ai.adapter);
        if settings.cache_limit_mb < before.cache_limit_mb {
            if let Err(e) = self
                .store
                .cache_trim(cache::limit_bytes(settings.cache_limit_mb))
            {
                tracing::debug!("cache: could not trim to the new limit ({e})");
            }
        }
        // A change of the accounts asks the CLIs again on the next read
        // (the user may have logged in to switch a host on) and builds the
        // providers for it from what they said last; any other change
        // keeps the providers as they are.
        let specs = if before.accounts != settings.accounts {
            self.accounts.mark_stale();
            let found = self
                .accounts
                .cached()
                .unwrap_or_else(accounts::Discovered::empty);
            accounts::provider_specs(&settings, &found)
        } else {
            self.specs()
        };
        self.rebuild_providers(specs, one_at_a_time);
        Ok(settings)
    }

    /// Put the providers for `specs` in place and restart a running poll
    /// loop with them. A spec that stays keeps its provider instance, with
    /// its rate-limit pause, conditional cache and user, as a fresh one
    /// would forget them; only new specs are built. `one_at_a_time` is the
    /// held `settings_update` lock: it is let go once the new loop is in
    /// place, before the old one's grace.
    fn rebuild_providers(&self, specs: Vec<ProviderSpec>, one_at_a_time: MutexGuard<'_, ()>) {
        let old = self
            .providers
            .lock()
            .expect("providers lock poisoned")
            .clone();
        let next: Vec<Built> = specs
            .into_iter()
            .filter_map(|spec| match old.iter().find(|b| b.spec == spec) {
                Some(kept) => Some(kept.clone()),
                None => (self.build_providers)(&spec).map(|provider| Built { spec, provider }),
            })
            .collect();
        *self.providers.lock().expect("providers lock poisoned") = next;
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

    /// Every tracked review of a host the user left on, most recently
    /// updated first (a host switched off keeps its reviews stored; they
    /// show again when it is switched back on). Filter on
    /// [`ReviewRecord::is_active`] and [`ReviewRecord::status`] for the tabs.
    pub fn reviews(&self) -> CoreResult<Vec<ReviewRecord>> {
        let settings = self.settings();
        Ok(self
            .store
            .reviews()?
            .into_iter()
            .filter(|r| settings.account_enabled(r.review.provider, &r.review.host))
            .collect())
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
        let providers = self.live_providers();
        let report = poll::refresh_all_unless(&providers, &self.store, &Utc::now, None)
            .expect("no stop flag to stop on");
        Self::note_refresh(&self.accounts, &polled(&providers), &report);
        report
    }

    /// Refresh one review now (blocking): its detail and threads from its
    /// provider, stored and classified as a poll would (see
    /// [`poll::refresh_review`]). Reads only. A refresh already under way is
    /// waited for first; one that read this review after the ask counts as
    /// this refresh. A rate-limited provider is not called: the error says
    /// until when. The review's cached file lists are dropped first (see
    /// [`cache`]), so its diff is read anew too.
    pub fn refresh_review(&self, id: ReviewId) -> CoreResult<ReviewRefresh> {
        let record = self.review(id)?;
        let provider = self.provider_for(&record)?;
        // Asked for fresh: the file lists are read from the platform again
        // (file contents at a commit cannot change, so they stay).
        if let Err(e) = self.store.cache_forget_diffs(&record.review) {
            tracing::debug!("cache: could not forget the review's file lists ({e})");
        }
        let refreshed = poll::refresh_review(provider.as_ref(), &self.store, id, &Utc::now);
        match &refreshed {
            Err(e) if e.is_unavailable() => self.accounts.mark_stale(),
            Err(_) => {}
            Ok(_) => self.accounts.note_reachable(provider.id(), provider.host()),
        }
        refreshed
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

    /// The provider a review belongs to: its host's. A host switched off,
    /// or without a login, has none.
    fn provider_for(&self, record: &ReviewRecord) -> CoreResult<Arc<dyn ReviewProvider>> {
        self.live_providers()
            .into_iter()
            .find(|p| p.id() == record.review.provider && p.host() == record.review.host)
            .ok_or_else(|| {
                CoreError::Provider(format!(
                    "{} at {} is not set up: check Settings → Accounts",
                    record.review.provider, record.review.host
                ))
            })
    }

    /// The content of `path` at `sha` in the review's repository (blocking):
    /// from the local clone when one is configured and has the commit, else
    /// from the disk cache, else from the provider (then cached).
    pub fn file_at(&self, id: ReviewId, sha: &str, path: &str) -> CoreResult<String> {
        let record = self.review(id)?;
        let clone = self.clone_of(&record);
        git::file_or(clone.as_ref(), sha, path, || {
            self.cached_file_at(&record, sha, path)
        })
    }

    /// The contents of several `(sha, path)` pairs of one review, in order,
    /// each as [`file_at`](Self::file_at) would read it (blocking): the
    /// local clone's in one `cat-file --batch` per commit, the cache's,
    /// then the provider's on up to [`FILE_READS`] threads. One failed read
    /// fails only its own entry.
    pub fn files_at(
        &self,
        id: ReviewId,
        wanted: &[(String, String)],
    ) -> CoreResult<Vec<CoreResult<String>>> {
        let record = self.review(id)?;
        let clone = self.clone_of(&record);
        let mut out: Vec<Option<CoreResult<String>>> = (0..wanted.len()).map(|_| None).collect();
        if let Some(clone) = &clone {
            let mut by_sha: HashMap<&str, Vec<usize>> = HashMap::new();
            for (i, (sha, _)) in wanted.iter().enumerate() {
                by_sha.entry(sha.as_str()).or_default().push(i);
            }
            for (sha, at) in by_sha {
                let paths: Vec<String> = at.iter().map(|&i| wanted[i].1.clone()).collect();
                // `None` as a whole: the commit is not here; the API reads them.
                let Some(found) = clone.files_at(sha, &paths) else {
                    continue;
                };
                for (&i, text) in at.iter().zip(found) {
                    out[i] = Some(match text {
                        Some(text) => Ok(text),
                        // Not a UTF-8 file here, or none: asked singly, which
                        // tells a missing file from one the API must read.
                        None => git::file_or(Some(clone), sha, &wanted[i].1, || {
                            self.cached_file_at(&record, sha, &wanted[i].1)
                        }),
                    });
                }
            }
        }
        let rest: Vec<usize> = (0..wanted.len()).filter(|&i| out[i].is_none()).collect();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let read: Mutex<Vec<(usize, CoreResult<String>)>> = Mutex::new(Vec::new());
        std::thread::scope(|s| {
            for _ in 0..FILE_READS.min(rest.len()) {
                s.spawn(|| loop {
                    let k = next.fetch_add(1, Ordering::SeqCst);
                    let Some(&i) = rest.get(k) else { break };
                    let (sha, path) = &wanted[i];
                    let text = self.cached_file_at(&record, sha, path);
                    read.lock().expect("reads lock poisoned").push((i, text));
                });
            }
        });
        for (i, text) in read.into_inner().expect("reads lock poisoned") {
            out[i] = Some(text);
        }
        Ok(out
            .into_iter()
            .map(|r| r.unwrap_or_else(|| Err(CoreError::Provider("a file read panicked".into()))))
            .collect())
    }

    /// The review's local clone, when one is set and opens.
    fn clone_of(&self, record: &ReviewRecord) -> Option<git::LocalClone> {
        self.settings()
            .clone_path(&record.review)
            .and_then(git::LocalClone::open)
    }

    /// `path` at `sha` from the disk cache, else from the provider (then
    /// cached). The clone is asked before this, never after: what it
    /// answers needs no cache.
    fn cached_file_at(&self, record: &ReviewRecord, sha: &str, path: &str) -> CoreResult<String> {
        let cache = self.disk_cache();
        if let Some(text) = cache.file(&record.review, sha, path) {
            return Ok(text);
        }
        let text = self
            .provider_for(record)?
            .file_at(&record.review, sha, path)?;
        cache.put_file(&record.review, sha, path, &text);
        Ok(text)
    }

    /// The disk cache, at the settings' limit.
    pub(crate) fn disk_cache(&self) -> cache::DiskCache<'_> {
        cache::DiskCache {
            store: &self.store,
            limit: cache::limit_bytes(self.settings().cache_limit_mb),
        }
    }

    /// One range of a review's diff: from the disk cache, else from the
    /// provider (then cached when it is of the commits the range names).
    pub(crate) fn cached_diff(
        &self,
        provider: &dyn ReviewProvider,
        record: &ReviewRecord,
        range: &provider::DiffRange,
    ) -> CoreResult<provider::FileDiff> {
        let cache = self.disk_cache();
        if let Some(diff) = cache.diff(&record.review, range) {
            return Ok(diff);
        }
        let diff = provider.fetch_files(&record.review, range)?;
        cache.put_diff(&record.review, range, &diff);
        Ok(diff)
    }

    /// What the disk cache holds.
    pub fn cache_stats(&self) -> CoreResult<cache::CacheStats> {
        self.store.cache_stats()
    }

    /// Empty the disk cache (Settings → General).
    pub fn clear_cache(&self) -> CoreResult<cache::CacheStats> {
        self.store.cache_clear()?;
        self.store.cache_stats()
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
        let fetch =
            |range: &provider::DiffRange| self.cached_diff(provider.as_ref(), &record, range);
        let (mut diff, keys, own) = files::fetch(&fetch, &record, since_last_review)?;
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
        let diff = self.cached_diff(
            self.provider_for(record)?.as_ref(),
            record,
            &files::review_range(record),
        )?;
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
        let book = self.accounts.clone();
        let hosts = polled(&providers);
        let poller = poll::Poller::start(providers, self.store.clone(), interval, move |r| {
            Self::note_refresh(&book, &hosts, r);
            cb(r)
        });
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
    /// loop is told to stop, submit scripts and script drafts are
    /// stopped, AI runs are cancelled and their agents killed
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
        // A submit script still running is killed (it would outlive Argus
        // otherwise); the ones waiting behind it never start.
        self.scripts.shutdown();
        self.drafts.shutdown();
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

/// The (provider, host) of each of `providers`.
fn polled(providers: &[Arc<dyn ReviewProvider>]) -> Vec<(provider::ProviderId, String)> {
    providers
        .iter()
        .map(|p| (p.id(), p.host().to_string()))
        .collect()
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
    use super::*;

    #[test]
    fn opening_a_core_never_rewrites_a_corrupt_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        // A hand edit gone wrong: a trailing comma.
        let broken = "{\n  \"pollIntervalSecs\": 60,\n}\n";
        std::fs::write(&path, broken).unwrap();
        let core = Core::open(&dir.path().join("argus.db"), &path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        // It runs on the defaults until the file is fixed or saved over.
        assert_eq!(core.settings(), Settings::default());
    }
}
