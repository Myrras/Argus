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

use super::job::JobContext;
use super::model::AiRunState;
use super::output::RunOutput;
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

#[derive(Default)]
struct Sched {
    /// Review → its run's cancel flag.
    running: HashMap<ReviewId, Arc<AtomicBool>>,
    /// At most one entry per review.
    queue: VecDeque<(ReviewId, JobContext)>,
}

struct Inner {
    store: Arc<Store>,
    executor: Mutex<Executor>,
    max_concurrent: usize,
    enabled: AtomicBool,
    sched: Mutex<Sched>,
    /// Signalled whenever a run ends (for `wait_idle`).
    changed: Condvar,
    callback: Mutex<Option<AiStateCallback>>,
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
                enabled: AtomicBool::new(enabled),
                sched: Mutex::new(Sched::default()),
                changed: Condvar::new(),
                callback: Mutex::new(None),
            }),
        }
    }

    #[cfg(test)]
    pub fn set_executor(&self, executor: Executor) {
        *self.inner.executor.lock().expect("executor lock poisoned") = executor;
    }

    pub fn enabled(&self) -> bool {
        self.inner.enabled.load(Ordering::Relaxed)
    }

    pub fn set_callback(&self, cb: AiStateCallback) {
        *self.inner.callback.lock().expect("callback lock poisoned") = Some(cb);
    }

    /// Turning the AI pass off drops the queue and cancels running runs.
    pub fn set_enabled(&self, enabled: bool) {
        self.inner.enabled.store(enabled, Ordering::Relaxed);
        if enabled {
            return;
        }
        let dropped: Vec<ReviewId> = {
            let mut s = self.inner.sched.lock().expect("sched lock poisoned");
            for flag in s.running.values() {
                flag.store(true, Ordering::Relaxed);
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
    /// fresher context). Returns false when the AI pass is off.
    pub fn enqueue(&self, review: ReviewId, ctx: JobContext) -> bool {
        if !self.enabled() {
            return false;
        }
        {
            let mut s = self.inner.sched.lock().expect("sched lock poisoned");
            match s.queue.iter_mut().find(|(id, _)| *id == review) {
                Some(entry) => entry.1 = ctx,
                None => s.queue.push_back((review, ctx)),
            }
        }
        self.inner.emit(review);
        Inner::pump(&self.inner);
        true
    }

    /// Drop a queued run of `review` and stop a running one (it ends
    /// `failed` with `cancelled`).
    pub fn cancel(&self, review: ReviewId) {
        let was_queued = {
            let mut s = self.inner.sched.lock().expect("sched lock poisoned");
            if let Some(flag) = s.running.get(&review) {
                flag.store(true, Ordering::Relaxed);
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
        s.queue.clear();
        for flag in s.running.values() {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

impl Inner {
    fn state(&self, review: ReviewId) -> CoreResult<AiRunState> {
        let enabled = self.enabled.load(Ordering::Relaxed);
        if !enabled {
            return Ok(AiRunState::not_started(review, false));
        }
        let queued = {
            let s = self.sched.lock().expect("sched lock poisoned");
            s.queue.iter().any(|(id, _)| *id == review)
        };
        Ok(match ai_store::run(&self.store, review)? {
            Some(row) => row.state(true, queued),
            None => AiRunState {
                queued,
                ..AiRunState::not_started(review, true)
            },
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
        while s.running.len() < this.max_concurrent {
            let Some(i) = s
                .queue
                .iter()
                .position(|(id, _)| !s.running.contains_key(id))
            else {
                break;
            };
            let (review, ctx) = s.queue.remove(i).expect("index in range");
            let flag = Arc::new(AtomicBool::new(false));
            s.running.insert(review, flag.clone());
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
            Ok(out) => ai_store::save_done(&self.store, review, record.round, &out, now),
            Err(e) => ai_store::mark_failed(&self.store, review, &e, now),
        };
        if let Err(e) = saved {
            tracing::warn!("AI run: could not store its result: {e}");
            let _ = ai_store::mark_failed(&self.store, review, "could not store the result", now);
        }
    }
}
