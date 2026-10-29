//! The submit scripts' queue: one worker thread at a time takes each
//! submit's batch in turn and runs its scripts one after the other, then
//! reports the batch. The thread starts with the first batch and ends when
//! the queue is empty.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::{Planned, ScriptContext, ScriptJob, ScriptResult, ScriptsDone, ScriptsDoneCallback};
use crate::ids::ReviewId;

/// What runs one script: the real process runner ([`super::run::execute`])
/// or a test's fake. It stops (killing the script) once the flag is set.
pub(crate) type Runner = Arc<dyn Fn(&ScriptJob, &AtomicBool) -> ScriptResult + Send + Sync>;

/// One submit's scripts.
pub(crate) struct Batch {
    pub review_id: ReviewId,
    pub context: ScriptContext,
    pub clone_dir: Option<PathBuf>,
    pub planned: Vec<Planned>,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Batch>,
    /// A worker thread is running (and will take what is queued).
    working: bool,
    shut_down: bool,
}

struct Inner {
    state: Mutex<State>,
    /// Signalled when the worker ends (for `wait_idle`).
    idle: Condvar,
    runner: Mutex<Runner>,
    callback: Mutex<Option<ScriptsDoneCallback>>,
    /// Set on shutdown: the script running is killed, the rest dropped.
    cancel: AtomicBool,
}

pub(crate) struct ScriptRuntime {
    inner: Arc<Inner>,
}

impl ScriptRuntime {
    pub fn new(runner: Runner) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State::default()),
                idle: Condvar::new(),
                runner: Mutex::new(runner),
                callback: Mutex::new(None),
                cancel: AtomicBool::new(false),
            }),
        }
    }

    #[cfg(test)]
    pub fn set_runner(&self, runner: Runner) {
        *self.inner.runner.lock().expect("runner lock poisoned") = runner;
    }

    pub fn set_callback(&self, cb: ScriptsDoneCallback) {
        *self.inner.callback.lock().expect("callback lock poisoned") = Some(cb);
    }

    /// Queue a submit's scripts; a worker starts when none runs. Nothing is
    /// queued after shutdown.
    pub fn enqueue(&self, batch: Batch) {
        let mut state = self.inner.state.lock().expect("scripts lock poisoned");
        if state.shut_down {
            return;
        }
        state.queue.push_back(batch);
        if state.working {
            return;
        }
        state.working = true;
        drop(state);
        let inner = self.inner.clone();
        std::thread::Builder::new()
            .name("argus-submit-scripts".into())
            .spawn(move || work(&inner))
            .map(drop)
            .unwrap_or_else(|_| {
                // No thread: nothing runs; the queue is dropped so a later
                // submit tries again.
                tracing::warn!("could not start the submit scripts' thread");
                let mut state = self.inner.state.lock().expect("scripts lock poisoned");
                state.queue.clear();
                state.working = false;
                self.inner.idle.notify_all();
            });
    }

    /// Wait until nothing is queued or running; `false` when `timeout`
    /// passed first.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.inner.state.lock().expect("scripts lock poisoned");
        while state.working || !state.queue.is_empty() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = self
                .inner
                .idle
                .wait_timeout(state, left)
                .expect("scripts lock poisoned")
                .0;
        }
        true
    }

    /// Quitting: what waits is dropped, the script running is killed, and
    /// nothing starts afterwards. Does not wait.
    pub fn shutdown(&self) {
        let mut state = self.inner.state.lock().expect("scripts lock poisoned");
        state.shut_down = true;
        state.queue.clear();
        self.inner.cancel.store(true, Ordering::SeqCst);
    }
}

/// The worker: batches in the order queued, until none is left.
fn work(inner: &Inner) {
    loop {
        let batch = {
            let mut state = inner.state.lock().expect("scripts lock poisoned");
            match state.queue.pop_front() {
                Some(b) => b,
                None => {
                    state.working = false;
                    inner.idle.notify_all();
                    return;
                }
            }
        };
        let done = run_batch(inner, batch);
        if inner.cancel.load(Ordering::SeqCst) {
            continue;
        }
        let cb = inner
            .callback
            .lock()
            .expect("callback lock poisoned")
            .clone();
        if let Some(cb) = cb {
            cb(&done);
        }
    }
}

fn run_batch(inner: &Inner, batch: Batch) -> ScriptsDone {
    let runner = inner.runner.lock().expect("runner lock poisoned").clone();
    let mut results = Vec::with_capacity(batch.planned.len());
    for planned in batch.planned {
        if inner.cancel.load(Ordering::SeqCst) {
            break;
        }
        let result = match planned {
            Planned::Skip(result) => result,
            Planned::Run(script) => {
                let job = ScriptJob {
                    script,
                    context: batch.context.clone(),
                    clone_dir: batch.clone_dir.clone(),
                };
                let result = runner(&job, &inner.cancel);
                // The status only: a script's output never goes to the log.
                tracing::debug!("submit script {}: {:?}", job.script.id, result.status);
                result
            }
        };
        results.push(result);
    }
    ScriptsDone {
        review_id: batch.review_id,
        results,
    }
}
