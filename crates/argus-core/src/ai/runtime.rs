//! Scheduling AI runs: one at a time per review, at most
//! [`MAX_CONCURRENT`](super::MAX_CONCURRENT) overall, FIFO. Each run is a
//! plain thread (the agent and the providers block, as in `poll`).
//!
//! Every state change (queued, running, done, failed, a queued run
//! cancelled) is reported through the `ai:state` callback.

use std::collections::{HashMap, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use chrono::Utc;

use super::adapter::{self, ChildSlot};
use super::job::{self, JobContext};
use super::model::AiRunState;
use super::output::{self, RunOutput};
use super::store as ai_store;
use super::CANCELLED;
use crate::error::CoreResult;
use crate::ids::ReviewId;
use crate::store::{ReviewRecord, Store};

/// What `on_ai_state` registers: called after every state change of a run.
pub type AiStateCallback = Arc<dyn Fn(&AiRunState) + Send + Sync>;

/// Runs one job (blocking). The real one is [`super::job::execute`]; tests
/// pass their own.
pub(crate) type Executor = Arc<
    dyn Fn(&Store, &ReviewRecord, &JobContext, &AtomicBool) -> Result<RunOutput, String>
        + Send
        + Sync,
>;

/// A started run: its cancel flag and its agent process.
struct Running {
    cancel: Arc<AtomicBool>,
    child: ChildSlot,
}

#[derive(Default)]
struct Sched {
    running: HashMap<ReviewId, Running>,
    /// At most one entry per review.
    queue: VecDeque<(ReviewId, JobContext)>,
    /// The AI pass is on. Read and written under this lock, so turning it
    /// off and queueing (or starting) a run never interleave.
    enabled: bool,
    /// [`AiRuntime::shutdown`] ran: nothing starts any more.
    stopped: bool,
}

struct Inner {
    store: Arc<Store>,
    executor: Mutex<Executor>,
    max_concurrent: usize,
    sched: Mutex<Sched>,
    /// Signalled whenever a run ends (for `wait_idle` and `shutdown`).
    changed: Condvar,
    callback: Mutex<Option<AiStateCallback>>,
    /// The configured adapter id, for `readOnlyGuaranteed`.
    adapter: Mutex<String>,
}

/// The AI scheduler a [`Core`](crate::Core) owns. Dropping it cancels what
/// is queued and running.
pub(crate) struct AiRuntime {
    inner: Arc<Inner>,
}

impl AiRuntime {
    /// A runtime over `store`. Runs left `running` by a previous process
    /// are marked failed (`interrupted`) first.
    pub fn new(
        store: Arc<Store>,
        enabled: bool,
        executor: Executor,
        max_concurrent: usize,
    ) -> Self {
        if let Err(e) = ai_store::interrupt_running(&store, Utc::now()) {
            tracing::warn!("could not mark interrupted AI runs: {e}");
        }
        Self {
            inner: Arc::new(Inner {
                store,
                executor: Mutex::new(executor),
                max_concurrent: max_concurrent.max(1),
                sched: Mutex::new(Sched {
                    enabled,
                    ..Sched::default()
                }),
                changed: Condvar::new(),
                callback: Mutex::new(None),
                adapter: Mutex::new(String::new()),
            }),
        }
    }

    /// The adapter the settings name (what `readOnlyGuaranteed` is about).
    pub fn set_adapter(&self, id: &str) {
        *self.inner.adapter.lock().expect("adapter lock poisoned") = id.trim().to_string();
    }

    #[cfg(test)]
    pub fn set_executor(&self, executor: Executor) {
        *self.inner.executor.lock().expect("executor lock poisoned") = executor;
    }

    pub fn enabled(&self) -> bool {
        self.inner.enabled()
    }

    pub fn set_callback(&self, cb: AiStateCallback) {
        *self.inner.callback.lock().expect("callback lock poisoned") = Some(cb);
    }

    /// Turning the AI pass off drops the queue and cancels running runs.
    pub fn set_enabled(&self, enabled: bool) {
        let dropped: Vec<ReviewId> = {
            let mut s = self.inner.sched.lock().expect("sched lock poisoned");
            s.enabled = enabled;
            if enabled {
                return;
            }
            for run in s.running.values() {
                run.cancel.store(true, Ordering::Relaxed);
            }
            s.queue.drain(..).map(|(id, _)| id).collect()
        };
        for id in dropped {
            self.inner.emit(id);
        }
    }

    /// Whether a run of `review` is running or queued.
    pub fn busy(&self, review: ReviewId) -> bool {
        let s = self.inner.sched.lock().expect("sched lock poisoned");
        s.running.contains_key(&review) || s.queue.iter().any(|(id, _)| *id == review)
    }

    /// Queue a run of `review` (a queued one keeps its place and takes the
    /// fresher context). Returns false when the AI pass is off (or Argus
    /// is shutting down).
    pub fn enqueue(&self, review: ReviewId, ctx: JobContext) -> bool {
        {
            let mut s = self.inner.sched.lock().expect("sched lock poisoned");
            if !s.enabled || s.stopped {
                return false;
            }
            match s.queue.iter_mut().find(|(id, _)| *id == review) {
                Some(entry) => entry.1 = ctx,
                None => s.queue.push_back((review, ctx)),
            }
        }
        self.inner.emit(review);
        Inner::pump(&self.inner);
        true
    }

    /// Quitting: nothing new starts, the queue is dropped, running runs are
    /// cancelled and their agents killed, then this waits up to `grace` for
    /// the run threads to finish (each removes its temp dir and stores its
    /// `cancelled` end). Whether they all did.
    pub fn shutdown(&self, grace: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + grace;
        let mut s = self.inner.sched.lock().expect("sched lock poisoned");
        s.stopped = true;
        s.queue.clear();
        for run in s.running.values() {
            run.cancel.store(true, Ordering::Relaxed);
            adapter::kill_child(&run.child);
        }
        while !s.running.is_empty() {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return false;
            }
            s = self
                .inner
                .changed
                .wait_timeout(s, left)
                .expect("sched lock poisoned")
                .0;
        }
        true
    }

    /// Drop a queued run of `review` and stop a running one (it ends
    /// `failed` with `cancelled`).
    pub fn cancel(&self, review: ReviewId) {
        let was_queued = {
            let mut s = self.inner.sched.lock().expect("sched lock poisoned");
            if let Some(run) = s.running.get(&review) {
                run.cancel.store(true, Ordering::Relaxed);
            }
            let before = s.queue.len();
            s.queue.retain(|(id, _)| *id != review);
            s.queue.len() != before
        };
        if was_queued {
            self.inner.emit(review);
        }
    }

    pub fn state(&self, review: ReviewId) -> CoreResult<AiRunState> {
        self.inner.state(review)
    }

    /// Put a run in the queue without the checks of [`enqueue`] (tests of
    /// the start path).
    #[cfg(test)]
    pub fn queue_unchecked(&self, review: ReviewId, ctx: JobContext) {
        let mut s = self.inner.sched.lock().expect("sched lock poisoned");
        s.queue.push_back((review, ctx));
    }

    /// Start what the queue holds and a slot allows.
    #[cfg(test)]
    pub fn pump(&self) {
        Inner::pump(&self.inner);
    }

    /// Block until nothing is queued or running, or `timeout` passes;
    /// whether it went idle.
    #[cfg(test)]
    pub fn wait_idle(&self, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let mut s = self.inner.sched.lock().expect("sched lock poisoned");
        while !(s.running.is_empty() && s.queue.is_empty()) {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return false;
            }
            s = self
                .inner
                .changed
                .wait_timeout(s, left)
                .expect("sched lock poisoned")
                .0;
        }
        true
    }
}

impl Drop for AiRuntime {
    fn drop(&mut self) {
        let mut s = self.inner.sched.lock().expect("sched lock poisoned");
        s.stopped = true;
        s.queue.clear();
        for run in s.running.values() {
            run.cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl Inner {
    fn enabled(&self) -> bool {
        self.sched.lock().expect("sched lock poisoned").enabled
    }

    fn state(&self, review: ReviewId) -> CoreResult<AiRunState> {
        let (enabled, queued) = {
            let s = self.sched.lock().expect("sched lock poisoned");
            (s.enabled, s.queue.iter().any(|(id, _)| *id == review))
        };
        let read_only =
            adapter::read_only_guaranteed(&self.adapter.lock().expect("adapter lock poisoned"));
        let state = if !enabled {
            AiRunState::not_started(review, false)
        } else {
            match ai_store::run(&self.store, review)? {
                Some(row) => row.state(true, queued),
                None => AiRunState {
                    queued,
                    ..AiRunState::not_started(review, true)
                },
            }
        };
        Ok(AiRunState {
            read_only_guaranteed: read_only,
            ..state
        })
    }

    fn emit(&self, review: ReviewId) {
        let cb = self
            .callback
            .lock()
            .expect("callback lock poisoned")
            .clone();
        if let Some(cb) = cb {
            match self.state(review) {
                Ok(state) => cb(&state),
                Err(e) => tracing::warn!("could not read an AI run state: {e}"),
            }
        }
    }

    /// Start queued runs while there is a free slot, oldest first, skipping
    /// reviews that already run.
    fn pump(this: &Arc<Self>) {
        let mut s = this.sched.lock().expect("sched lock poisoned");
        // Checked under the lock, against a `set_enabled(false)` racing the
        // enqueue that led here.
        if !s.enabled || s.stopped {
            return;
        }
        while s.running.len() < this.max_concurrent {
            let Some(i) = s
                .queue
                .iter()
                .position(|(id, _)| !s.running.contains_key(id))
            else {
                break;
            };
            let (review, mut ctx) = s.queue.remove(i).expect("index in range");
            let flag = Arc::new(AtomicBool::new(false));
            ctx.child = ChildSlot::default();
            s.running.insert(
                review,
                Running {
                    cancel: flag.clone(),
                    child: ctx.child.clone(),
                },
            );
            let inner = Arc::clone(this);
            let spawned = std::thread::Builder::new()
                .name("argus-ai".into())
                .spawn(move || inner.work(review, ctx, flag));
            if spawned.is_err() {
                s.running.remove(&review);
                tracing::warn!("could not start an AI run thread");
            }
        }
    }

    fn work(self: Arc<Self>, review: ReviewId, ctx: JobContext, cancel: Arc<AtomicBool>) {
        self.run_one(review, &ctx, &cancel);
        // Reported before the slot is freed, so whoever waits for idle has
        // seen the final state.
        self.emit(review);
        {
            let mut s = self.sched.lock().expect("sched lock poisoned");
            s.running.remove(&review);
        }
        self.changed.notify_all();
        Inner::pump(&self);
    }

    fn run_one(&self, review: ReviewId, ctx: &JobContext, cancel: &AtomicBool) {
        let record = match self.store.review(review) {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!("AI run: could not read the review: {e}");
                return;
            }
        };
        let adapter = ctx.settings.ai.adapter.trim().to_string();
        if let Err(e) = ai_store::mark_running(
            &self.store,
            review,
            record.round,
            &record.head_sha,
            &adapter,
            Utc::now(),
        ) {
            tracing::warn!("AI run: could not record its start: {e}");
            return;
        }
        self.emit(review);
        let executor = self
            .executor
            .lock()
            .expect("executor lock poisoned")
            .clone();
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            executor(&self.store, &record, ctx, cancel)
        }))
        .unwrap_or_else(|_| Err("the AI run crashed".into()));
        let now = Utc::now();
        let saved = match result {
            _ if cancel.load(Ordering::Relaxed) => {
                ai_store::mark_failed(&self.store, review, CANCELLED, now)
            }
            Ok(mut out) => {
                // Whatever secret the agent quoted from a file it read stays
                // out of the store, and so off the platform.
                output::redact(&mut out, &job::known_secrets(&ctx.settings));
                ai_store::save_done(&self.store, review, record.round, &out, now)
            }
            Err(e) => ai_store::mark_failed(&self.store, review, &e, now),
        };
        if let Err(e) = saved {
            tracing::warn!("AI run: could not store its result: {e}");
            let _ = ai_store::mark_failed(&self.store, review, "could not store the result", now);
        }
    }
}
