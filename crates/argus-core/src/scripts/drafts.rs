//! Script drafts: "Write with AI" as a conversation that runs in the
//! background.
//!
//! A draft is one conversation: the user's requests in order, each with the
//! script the agent wrote for it (or why none came back). Sending a request
//! returns at once with the draft `running`; the agent works on its own
//! thread and every status change goes to [`Core::on_script_draft`] (the
//! shell's `script-draft:state`). Each turn's prompt carries the whole
//! conversation (see [`generate::prompt`]), so a follow-up keeps the earlier
//! requests as context and gets a whole updated script back.
//!
//! Drafts live in memory for the app's lifetime; quitting drops them.
//! Drafts are independent: at most [`MAX_RUNNING`] ask the agent at once,
//! the rest wait their turn (`running` with `waiting` set). Only the user
//! stops one ([`Core::cancel_script_draft`], or discarding it); a new
//! request never stops another draft. Nothing here saves a script, runs
//! one or writes to a platform: saving stays [`Core::save_submit_script`].

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::attach::{self, AttachContext, Attachment, KnownHost};
use super::generate::{self, GeneratedScript, Generator, Handle, Report, Update};
use crate::ai::adapter::StreamEvent;
use crate::error::{CoreError, CoreResult};
use crate::settings::AiSettings;
use crate::Core;

/// How many drafts ask the agent at once; the rest wait.
pub const MAX_RUNNING: usize = 2;

/// The longest request the user can send.
pub const MAX_MESSAGE: usize = 8_000;

/// The longest title made from a request.
const MAX_TITLE: usize = 60;

/// A turn's error when the user stopped it (the prompt tells the agent).
pub(crate) const STOPPED: &str = "Stopped.";

/// Most characters of thinking a running turn keeps (its end).
pub const MAX_THINKING: usize = 20_000;

/// Least time between two `script-draft:progress` reports of one draft; a
/// stage change reports at once.
pub const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// Where a draft is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScriptDraftStatus {
    /// Nothing under way: the last turn has its script (or is a seed).
    Idle,
    /// The agent works on the last turn (or it waits for a free slot).
    Running,
    /// The last turn failed; `error` says why.
    Failed,
    /// The user stopped the last turn.
    Cancelled,
}

/// One request of the user's and what came back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftTurn {
    pub prompt: String,
    pub at: DateTime<Utc>,
    /// The whole script the agent wrote for it; `None` while running, or
    /// when it failed or was stopped.
    pub result: Option<GeneratedScript>,
    /// Why no script came back (Argus' words).
    pub error: Option<String>,
    /// What Argus attached for it ("MR !4242", "Code_review.md").
    #[serde(default)]
    pub attachments: Vec<String>,
    /// What it named but Argus did not attach, and why.
    #[serde(default)]
    pub notes: Vec<String>,
    /// The attached material itself, for this and later prompts (not on
    /// the wire).
    #[serde(skip)]
    pub(crate) material: Vec<Attachment>,
}

impl DraftTurn {
    /// A request just sent.
    pub fn new(prompt: &str) -> Self {
        Self {
            prompt: prompt.trim().to_string(),
            at: Utc::now(),
            result: None,
            error: None,
            attachments: Vec::new(),
            notes: Vec::new(),
            material: Vec::new(),
        }
    }
}

/// Where a running turn is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DraftStage {
    /// Fetching the links and reading the files the request names.
    Reading,
    /// The agent started; nothing streamed yet.
    Starting,
    Thinking,
    /// The answer is coming in.
    Writing,
    /// Argus checks the answer.
    Validating,
    /// The answer was not usable: the agent is asked once more.
    Repairing,
}

/// What the agent of a running turn is doing, for the writer. Gone once
/// the turn ends: the thinking is not kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptDraftProgress {
    /// The running turn's index.
    pub turn: usize,
    pub stage: DraftStage,
    /// When the turn got its slot (elapsed time counts from here).
    pub started_at: DateTime<Utc>,
    /// The end of the thinking so far (at most [`MAX_THINKING`]
    /// characters, token shapes redacted); empty when the agent hides it.
    pub thinking: String,
    /// The thinking was longer: its start is left out.
    pub thinking_cut: bool,
    /// Claude's estimate of thinking tokens so far.
    pub thinking_tokens: Option<u64>,
    /// How much of the answer came in.
    pub answer_chars: usize,
    /// Something worth saying, e.g. that the API is busy and Claude retries.
    pub notice: Option<String>,
}

/// One `script-draft:progress` report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptDraftProgressEvent {
    pub draft_id: String,
    pub progress: ScriptDraftProgress,
}

/// What [`Core::on_script_draft_progress`] registers.
pub type ScriptDraftProgressCallback = Arc<dyn Fn(&ScriptDraftProgressEvent) + Send + Sync>;

/// Lets a report through at most every `every` (see [`PROGRESS_EVERY`]).
#[derive(Debug, Clone)]
pub(crate) struct Throttle {
    every: Duration,
    last: Option<Instant>,
}

impl Throttle {
    pub(crate) fn new(every: Duration) -> Self {
        Self { every, last: None }
    }

    /// Whether a report may go out at `now` (and if so, it counts as sent).
    pub(crate) fn ready(&mut self, now: Instant) -> bool {
        match self.last {
            Some(last) if now.saturating_duration_since(last) < self.every => false,
            _ => {
                self.last = Some(now);
                true
            }
        }
    }

    /// A report went out regardless (a stage change).
    pub(crate) fn sent(&mut self, now: Instant) {
        self.last = Some(now);
    }
}

impl ScriptDraftProgress {
    fn new(turn: usize, stage: DraftStage) -> Self {
        Self {
            turn,
            stage,
            started_at: Utc::now(),
            thinking: String::new(),
            thinking_cut: false,
            thinking_tokens: None,
            answer_chars: 0,
            notice: None,
        }
    }

    /// Take one update; whether the stage changed (reported at once).
    pub(crate) fn apply(&mut self, update: &Update) -> bool {
        let before = self.stage;
        match update {
            Update::Stage(stage) => self.stage = *stage,
            Update::Stream(StreamEvent::Thinking(t)) => {
                if self.stage == DraftStage::Starting {
                    self.stage = DraftStage::Thinking;
                }
                self.notice = None;
                self.thinking.push_str(t);
                let n = self.thinking.chars().count();
                if n > MAX_THINKING {
                    self.thinking = self.thinking.chars().skip(n - MAX_THINKING).collect();
                    self.thinking_cut = true;
                }
            }
            Update::Stream(StreamEvent::ThinkingTokens(n)) => {
                if self.stage == DraftStage::Starting {
                    self.stage = DraftStage::Thinking;
                }
                self.notice = None;
                self.thinking_tokens = Some(*n);
            }
            Update::Stream(StreamEvent::Text(t)) => {
                if matches!(self.stage, DraftStage::Starting | DraftStage::Thinking) {
                    self.stage = DraftStage::Writing;
                }
                self.notice = None;
                self.answer_chars += t.chars().count();
            }
            Update::Stream(StreamEvent::Retrying {
                attempt,
                max,
                status,
            }) => {
                let of = max.map(|m| format!(" of {m}")).unwrap_or_default();
                let why = match status {
                    Some(429) => "rate limited".to_string(),
                    Some(529) => "overloaded".to_string(),
                    Some(s) => format!("error {s}"),
                    None => "no response".to_string(),
                };
                self.notice = Some(format!(
                    "The API did not answer ({why}); the agent retries (attempt {attempt}{of})."
                ));
            }
        }
        if let Update::Stage(DraftStage::Repairing) = update {
            // A second ask streams afresh.
            self.answer_chars = 0;
        }
        self.stage != before
    }

    /// The copy that leaves Argus: token shapes in the thinking redacted.
    fn redacted(&self) -> Self {
        let mut out = self.clone();
        out.thinking = crate::ai::output::redact_text(&out.thinking, &[]);
        out
    }
}

/// One "Write with AI" conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptDraft {
    /// A UUID string.
    pub id: String,
    /// The submit script it changes: set by "Edit with AI", and once the
    /// draft is saved ([`Core::link_script_draft`]).
    pub script_id: Option<String>,
    /// The latest script's name, else the saved script's, else the first
    /// request cut short.
    pub title: String,
    /// "Edit with AI": the saved script the conversation started from.
    pub base: Option<GeneratedScript>,
    pub turns: Vec<DraftTurn>,
    pub status: ScriptDraftStatus,
    /// Running, but waiting for one of the [`MAX_RUNNING`] slots.
    pub waiting: bool,
    /// Why the last turn failed.
    pub error: Option<String>,
    /// While a turn runs (not while it waits): what the agent is doing.
    #[serde(default)]
    pub progress: Option<ScriptDraftProgress>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Goes up with every change (across all drafts), so the webview can
    /// tell an older copy from a newer one whatever order they arrive in.
    pub revision: u64,
}

impl ScriptDraft {
    /// The newest script: the last turn's that has one, else the base.
    pub fn latest(&self) -> Option<&GeneratedScript> {
        self.turns
            .iter()
            .rev()
            .find_map(|t| t.result.as_ref())
            .or(self.base.as_ref())
    }

    fn retitle(&mut self) {
        self.title = match self.latest() {
            Some(g) if !g.name.trim().is_empty() => g.name.trim().to_string(),
            _ => self
                .turns
                .first()
                .map(|t| short(&t.prompt))
                .unwrap_or_else(|| "New script".into()),
        };
    }
}

/// `text` on one line, at most [`MAX_TITLE`] characters.
fn short(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= MAX_TITLE {
        return line;
    }
    let cut: String = line.chars().take(MAX_TITLE - 1).collect();
    format!("{}…", cut.trim_end())
}

/// What [`Core::on_script_draft`] registers: called with the whole draft on
/// every change of its status (and when it waits, starts or is linked).
pub type ScriptDraftCallback = Arc<dyn Fn(&ScriptDraft) + Send + Sync>;

/// What a turn asks the agent with, fixed when it is sent.
#[derive(Clone)]
pub(crate) struct Agent {
    pub ai: AiSettings,
    pub hints: Vec<String>,
    pub generator: Generator,
    /// Where the request's links and paths are looked up.
    pub context: AttachContext,
}

struct Entry {
    draft: ScriptDraft,
    /// Set while the agent works on it.
    handle: Option<Arc<Handle>>,
    /// The agent of the turn that waits for a slot.
    pending: Option<Agent>,
    /// Paces the running turn's progress reports.
    throttle: Throttle,
}

#[derive(Default)]
struct State {
    drafts: Vec<Entry>,
    /// Ids waiting for a slot, in the order sent.
    queue: VecDeque<String>,
    running: usize,
    shut_down: bool,
}

struct Inner {
    state: Mutex<State>,
    /// Signalled when a run ends (for `wait_idle`).
    idle: Condvar,
    callback: Mutex<Option<ScriptDraftCallback>>,
    progress_callback: Mutex<Option<ScriptDraftProgressCallback>>,
    revision: AtomicU64,
}

impl Inner {
    fn touch(&self, draft: &mut ScriptDraft) {
        draft.updated_at = Utc::now();
        draft.revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
    }

    /// Report a change. Called with the state locked, so the events of one
    /// draft go out in the order they happened; the callback must not call
    /// back into the drafts (the shell's only emits).
    fn emit(&self, draft: &ScriptDraft) {
        let cb = self
            .callback
            .lock()
            .expect("draft callback lock poisoned")
            .clone();
        if let Some(cb) = cb {
            cb(draft);
        }
    }

    /// Report a running draft's progress (with the state locked, as
    /// [`emit`](Self::emit)).
    fn emit_progress(&self, draft: &ScriptDraft) {
        let Some(progress) = &draft.progress else {
            return;
        };
        let cb = self
            .progress_callback
            .lock()
            .expect("draft callback lock poisoned")
            .clone();
        if let Some(cb) = cb {
            cb(&ScriptDraftProgressEvent {
                draft_id: draft.id.clone(),
                progress: progress.clone(),
            });
        }
    }
}

/// The drafts and their runs.
pub(crate) struct DraftRuntime {
    inner: Arc<Inner>,
}

type Started = (String, Arc<Handle>, Agent);

impl DraftRuntime {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State::default()),
                idle: Condvar::new(),
                callback: Mutex::new(None),
                progress_callback: Mutex::new(None),
                revision: AtomicU64::new(0),
            }),
        }
    }

    pub fn set_callback(&self, cb: ScriptDraftCallback) {
        *self
            .inner
            .callback
            .lock()
            .expect("draft callback lock poisoned") = Some(cb);
    }

    pub fn set_progress_callback(&self, cb: ScriptDraftProgressCallback) {
        *self
            .inner
            .progress_callback
            .lock()
            .expect("draft callback lock poisoned") = Some(cb);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.state.lock().expect("drafts lock poisoned")
    }

    pub fn list(&self) -> Vec<ScriptDraft> {
        self.lock().drafts.iter().map(|e| e.draft.clone()).collect()
    }

    /// A new draft whose first turn is `prompt`, queued.
    pub fn start(
        &self,
        prompt: &str,
        script_id: Option<String>,
        base: Option<GeneratedScript>,
        agent: Agent,
    ) -> CoreResult<ScriptDraft> {
        let now = Utc::now();
        let mut draft = ScriptDraft {
            id: uuid::Uuid::new_v4().to_string(),
            script_id,
            title: String::new(),
            base,
            turns: Vec::new(),
            status: ScriptDraftStatus::Idle,
            waiting: false,
            error: None,
            progress: None,
            created_at: now,
            updated_at: now,
            revision: 0,
        };
        draft.retitle();
        let mut state = self.lock();
        if state.shut_down {
            return Err(CoreError::Cancelled);
        }
        let id = draft.id.clone();
        state.drafts.push(Entry {
            draft,
            handle: None,
            pending: None,
            throttle: Throttle::new(PROGRESS_EVERY),
        });
        self.enqueue(state, &id, Some(prompt), agent)
    }

    /// A follow-up turn on draft `id`; refused while it runs.
    pub fn send(&self, id: &str, prompt: &str, agent: Agent) -> CoreResult<ScriptDraft> {
        let state = self.lock();
        if state.shut_down {
            return Err(CoreError::Cancelled);
        }
        let entry = state
            .drafts
            .iter()
            .find(|e| e.draft.id == id)
            .ok_or(CoreError::NotFound)?;
        if entry.draft.status == ScriptDraftStatus::Running {
            return Err(CoreError::Conflict(
                "The AI is still writing this draft: wait for it or stop it first.".into(),
            ));
        }
        self.enqueue(state, id, Some(prompt), agent)
    }

    /// Send draft `id`'s last request again, after it failed or was
    /// stopped; refused while it runs or when it has its script.
    pub fn retry(&self, id: &str, agent: Agent) -> CoreResult<ScriptDraft> {
        let state = self.lock();
        if state.shut_down {
            return Err(CoreError::Cancelled);
        }
        let entry = state
            .drafts
            .iter()
            .find(|e| e.draft.id == id)
            .ok_or(CoreError::NotFound)?;
        match entry.draft.status {
            ScriptDraftStatus::Running => {
                return Err(CoreError::Conflict(
                    "The AI is still writing this draft: wait for it or stop it first.".into(),
                ))
            }
            ScriptDraftStatus::Idle => {
                return Err(CoreError::Conflict(
                    "This request has its script already: ask for changes instead.".into(),
                ))
            }
            ScriptDraftStatus::Failed | ScriptDraftStatus::Cancelled => {}
        }
        if entry.draft.turns.last().is_none_or(|t| t.result.is_some()) {
            return Err(CoreError::Conflict(
                "There is no request to send again.".into(),
            ));
        }
        self.enqueue(state, id, None, agent)
    }

    /// Add `prompt` as the draft's new turn (`None`: send the last one
    /// again), mark it running and waiting, queue it, start what fits,
    /// report it.
    fn enqueue(
        &self,
        mut state: std::sync::MutexGuard<'_, State>,
        id: &str,
        prompt: Option<&str>,
        agent: Agent,
    ) -> CoreResult<ScriptDraft> {
        let entry = state
            .drafts
            .iter_mut()
            .find(|e| e.draft.id == id)
            .ok_or(CoreError::NotFound)?;
        let d = &mut entry.draft;
        match prompt {
            Some(p) => d.turns.push(DraftTurn::new(p)),
            None => {
                if let Some(t) = d.turns.last_mut() {
                    t.error = None;
                    t.at = Utc::now();
                }
            }
        }
        d.status = ScriptDraftStatus::Running;
        d.waiting = true;
        d.error = None;
        d.progress = None;
        if d.turns.len() == 1 {
            d.retitle();
        }
        entry.pending = Some(agent);
        self.inner.touch(&mut entry.draft);
        state.queue.push_back(id.to_string());
        let started = self.pump(&mut state);
        // The new draft's state (running, waiting or not) once, then the
        // others that just started.
        let snapshot = state
            .drafts
            .iter()
            .find(|e| e.draft.id == id)
            .map(|e| e.draft.clone())
            .ok_or(CoreError::NotFound)?;
        if !started.iter().any(|s| s.0 == id) {
            self.inner.emit(&snapshot);
        }
        drop(state);
        self.spawn_all(started);
        Ok(snapshot)
    }

    /// Start waiting drafts while a slot is free: each one's `waiting` flag
    /// goes off and is reported. The threads are the caller's to spawn
    /// (after unlocking).
    fn pump(&self, state: &mut State) -> Vec<Started> {
        let mut started = Vec::new();
        while state.running < MAX_RUNNING {
            let Some(id) = state.queue.pop_front() else {
                break;
            };
            let Some(entry) = state.drafts.iter_mut().find(|e| e.draft.id == id) else {
                continue;
            };
            let Some(agent) = entry.pending.take() else {
                continue;
            };
            let handle = Arc::new(Handle::default());
            entry.handle = Some(handle.clone());
            entry.draft.waiting = false;
            let turn = entry.draft.turns.len().saturating_sub(1);
            let reading = entry
                .draft
                .turns
                .last()
                .is_some_and(|t| attach::mentions_anything(&t.prompt));
            entry.draft.progress = Some(ScriptDraftProgress::new(
                turn,
                if reading {
                    DraftStage::Reading
                } else {
                    DraftStage::Starting
                },
            ));
            entry.throttle = Throttle::new(PROGRESS_EVERY);
            self.inner.touch(&mut entry.draft);
            self.inner.emit(&entry.draft);
            state.running += 1;
            started.push((id, handle, agent));
        }
        started
    }

    fn spawn_all(&self, started: Vec<Started>) {
        for (id, handle, agent) in started {
            let inner = self.inner.clone();
            let spawned = std::thread::Builder::new()
                .name("argus-script-draft".into())
                .spawn({
                    let (id, handle) = (id.clone(), handle.clone());
                    move || {
                        let result = run_turn(&inner, &handle, &agent);
                        finish(&inner, &id, &handle, result);
                    }
                });
            if spawned.is_err() {
                tracing::warn!("could not start a script draft's thread");
                finish(
                    &self.inner,
                    &id,
                    &handle,
                    Err(CoreError::Agent(
                        "Couldn't write the script: Argus could not start the agent.".into(),
                    )),
                );
            }
        }
    }

    /// Stop draft `id`'s turn: a waiting one at once, a running one's agent
    /// is killed (its thread then reports `cancelled`). A no-op for a draft
    /// that does not run.
    pub fn cancel(&self, id: &str) -> CoreResult<ScriptDraft> {
        let mut state = self.lock();
        let State { drafts, queue, .. } = &mut *state;
        let entry = drafts
            .iter_mut()
            .find(|e| e.draft.id == id)
            .ok_or(CoreError::NotFound)?;
        if entry.draft.status == ScriptDraftStatus::Running {
            if let Some(h) = &entry.handle {
                h.cancel();
            } else {
                queue.retain(|q| q != id);
                entry.pending = None;
                stopped(&mut entry.draft);
                self.inner.touch(&mut entry.draft);
                self.inner.emit(&entry.draft);
            }
        }
        Ok(entry.draft.clone())
    }

    /// Link draft `id` to the submit script `script_id` (saved from it).
    pub fn link(&self, id: &str, script_id: &str) -> CoreResult<ScriptDraft> {
        let mut state = self.lock();
        let entry = state
            .drafts
            .iter_mut()
            .find(|e| e.draft.id == id)
            .ok_or(CoreError::NotFound)?;
        entry.draft.script_id = Some(script_id.to_string());
        self.inner.touch(&mut entry.draft);
        self.inner.emit(&entry.draft);
        Ok(entry.draft.clone())
    }

    /// Forget draft `id`, stopping its turn first. Reports nothing: the
    /// caller drops it.
    pub fn discard(&self, id: &str) -> CoreResult<()> {
        let mut state = self.lock();
        let at = state
            .drafts
            .iter()
            .position(|e| e.draft.id == id)
            .ok_or(CoreError::NotFound)?;
        let entry = state.drafts.remove(at);
        state.queue.retain(|q| q != id);
        if let Some(h) = entry.handle {
            h.cancel();
        }
        Ok(())
    }

    /// Wait until no draft runs or waits; `false` when `timeout` passed.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        while state.running > 0 || !state.queue.is_empty() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = self
                .inner
                .idle
                .wait_timeout(state, left)
                .expect("drafts lock poisoned")
                .0;
        }
        true
    }

    /// Quitting: every running agent is killed, waiting turns are dropped,
    /// nothing is reported or started afterwards. Does not wait.
    pub fn shutdown(&self) {
        let mut state = self.lock();
        state.shut_down = true;
        state.queue.clear();
        for entry in &mut state.drafts {
            entry.pending = None;
            if let Some(h) = &entry.handle {
                h.cancel();
            } else if entry.draft.status == ScriptDraftStatus::Running {
                stopped(&mut entry.draft);
            }
        }
        self.inner.idle.notify_all();
    }
}

/// One turn on its own thread: attach what the request names, build the
/// prompt from the conversation, ask the agent with progress reported.
fn run_turn(
    inner: &Arc<Inner>,
    handle: &Arc<Handle>,
    agent: &Agent,
) -> CoreResult<GeneratedScript> {
    let rt = DraftRuntime {
        inner: inner.clone(),
    };
    // The entry is found by its handle: a discarded draft has none.
    let with_entry = |f: &mut dyn FnMut(&mut Entry, &Inner)| {
        let mut state = rt.lock();
        if state.shut_down {
            return false;
        }
        match state
            .drafts
            .iter_mut()
            .find(|e| e.handle.as_ref().is_some_and(|h| Arc::ptr_eq(h, handle)))
        {
            Some(entry) => {
                f(entry, inner);
                true
            }
            None => false,
        }
    };
    let mut request = None;
    with_entry(&mut |e, _| request = e.draft.turns.last().map(|t| t.prompt.clone()));
    let Some(request) = request else {
        return Err(CoreError::Cancelled);
    };
    let gathered = if attach::mentions_anything(&request) {
        attach::gather(&request, &agent.context)
    } else {
        attach::Gathered::default()
    };
    if handle.cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(CoreError::Cancelled);
    }
    let mut snapshot = None;
    with_entry(&mut |e, inner| {
        if let Some(t) = e.draft.turns.last_mut() {
            t.attachments = gathered.material.iter().map(|a| a.label.clone()).collect();
            t.notes = gathered.notes.clone();
            t.material = gathered.material.clone();
        }
        if !gathered.material.is_empty() || !gathered.notes.is_empty() {
            inner.touch(&mut e.draft);
            inner.emit(&e.draft);
        }
        snapshot = Some((e.draft.base.clone(), e.draft.turns.clone()));
    });
    let Some((base, turns)) = snapshot else {
        return Err(CoreError::Cancelled);
    };
    let text = generate::prompt(base.as_ref(), &turns, &agent.hints);
    let report: Report = {
        let inner = inner.clone();
        let handle = handle.clone();
        Arc::new(move |update: Update| {
            let rt = DraftRuntime {
                inner: inner.clone(),
            };
            let mut state = rt.lock();
            if state.shut_down {
                return;
            }
            let Some(entry) = state
                .drafts
                .iter_mut()
                .find(|e| e.handle.as_ref().is_some_and(|h| Arc::ptr_eq(h, &handle)))
            else {
                return;
            };
            let Some(progress) = entry.draft.progress.as_mut() else {
                return;
            };
            let changed = progress.apply(&update);
            let now = Instant::now();
            let send = if changed {
                entry.throttle.sent(now);
                true
            } else {
                entry.throttle.ready(now)
            };
            if send {
                let mut copy = entry.draft.clone();
                copy.progress = copy.progress.map(|p| p.redacted());
                inner.emit_progress(&copy);
            }
        })
    };
    generate::generate(&agent.ai, &text, &agent.generator, handle, &report)
}

/// The last turn stopped by the user.
fn stopped(draft: &mut ScriptDraft) {
    draft.status = ScriptDraftStatus::Cancelled;
    draft.waiting = false;
    draft.error = None;
    draft.progress = None;
    if let Some(turn) = draft.turns.last_mut() {
        turn.error = Some(STOPPED.into());
    }
}

/// A run ended: put the result on the draft (unless it was discarded),
/// report it, start what waits.
fn finish(inner: &Arc<Inner>, id: &str, handle: &Arc<Handle>, result: CoreResult<GeneratedScript>) {
    let rt = DraftRuntime {
        inner: inner.clone(),
    };
    let mut state = rt.lock();
    state.running = state.running.saturating_sub(1);
    let shut_down = state.shut_down;
    if let Some(entry) = state
        .drafts
        .iter_mut()
        .find(|e| e.handle.as_ref().is_some_and(|h| Arc::ptr_eq(h, handle)))
    {
        entry.handle = None;
        let d = &mut entry.draft;
        d.waiting = false;
        // The thinking is not kept past its turn.
        d.progress = None;
        match result {
            Ok(g) => {
                d.status = ScriptDraftStatus::Idle;
                d.error = None;
                if let Some(turn) = d.turns.last_mut() {
                    turn.result = Some(g);
                    turn.error = None;
                }
                d.retitle();
            }
            Err(CoreError::Cancelled) => stopped(d),
            Err(e) => {
                let message = e.to_string();
                d.status = ScriptDraftStatus::Failed;
                d.error = Some(message.clone());
                if let Some(turn) = d.turns.last_mut() {
                    turn.error = Some(message);
                }
            }
        }
        inner.touch(&mut entry.draft);
        if !shut_down {
            inner.emit(&entry.draft);
        }
        // The status only: a script never goes to the log.
        tracing::debug!("script draft {id}: {:?}", entry.draft.status);
    }
    let started = if shut_down {
        Vec::new()
    } else {
        rt.pump(&mut state)
    };
    inner.idle.notify_all();
    drop(state);
    rt.spawn_all(started);
}

impl Core {
    /// What a turn asks the agent with now; an error with the reason when
    /// "Write with AI" cannot run (blocking: finds the agent).
    fn draft_agent(&self) -> CoreResult<Agent> {
        let settings = self.settings();
        if let Some(reason) = generate::status(&settings.ai).reason {
            return Err(CoreError::Agent(reason));
        }
        let mut roots: Vec<std::path::PathBuf> = settings.clone_paths.values().cloned().collect();
        roots.extend(self.submit_scripts_dir());
        let providers = self.providers();
        // The CLIs' last answer, never asked here: a link to a host without
        // a provider only needs the right note.
        let discovered = self
            .accounts
            .cached()
            .unwrap_or_else(crate::accounts::Discovered::empty);
        let hosts = crate::accounts::list(&settings, &discovered)
            .into_iter()
            .filter(|a| !a.host.is_empty())
            .map(|a| KnownHost {
                provider: a.provider,
                host: a.host,
                enabled: a.enabled,
            })
            .collect();
        Ok(Agent {
            hints: generate::platform_hints(&crate::polled(&providers)),
            context: AttachContext {
                providers,
                hosts,
                roots,
                home: crate::git::discover::home_dir(),
            },
            ai: settings.ai,
            generator: self
                .generator
                .lock()
                .expect("generator lock poisoned")
                .clone(),
        })
    }

    /// A new script draft whose first request is `description`, running in
    /// the background; returns at once with it `running` (or `waiting`).
    /// With `script_id` ("Edit with AI") the conversation starts from that
    /// saved script (read as [`read_submit_script`](Self::read_submit_script)
    /// does: only one inside Argus' scripts folder) and is linked to it.
    /// Writes nothing, runs nothing, never touches a platform.
    pub fn start_script_draft(
        &self,
        description: &str,
        script_id: Option<&str>,
    ) -> CoreResult<ScriptDraft> {
        let description = message(description)?;
        let base = match script_id.map(str::trim).filter(|s| !s.is_empty()) {
            None => None,
            Some(id) => {
                let content = self.read_submit_script(id)?;
                let saved = self
                    .settings()
                    .submit_scripts
                    .into_iter()
                    .find(|s| s.id == id)
                    .ok_or(CoreError::NotFound)?;
                Some(GeneratedScript {
                    name: saved.display_name(),
                    verdicts: saved.verdicts,
                    script: content,
                    explanation: String::new(),
                })
            }
        };
        let agent = self.draft_agent()?;
        let script_id = base.as_ref().and(script_id.map(|s| s.trim().to_string()));
        self.drafts.start(description, script_id, base, agent)
    }

    /// A follow-up request on draft `draft_id`, with every earlier request
    /// and the latest script as context; refused (`conflict`) while the
    /// draft runs. Returns at once, as [`start_script_draft`](Self::start_script_draft).
    pub fn send_script_draft(&self, draft_id: &str, text: &str) -> CoreResult<ScriptDraft> {
        let text = message(text)?;
        let agent = self.draft_agent()?;
        self.drafts.send(draft_id.trim(), text, agent)
    }

    /// Send draft `draft_id`'s last request again after it failed or was
    /// stopped (Retry): the same text, its links and files attached
    /// afresh. Returns at once, as [`start_script_draft`](Self::start_script_draft).
    pub fn retry_script_draft(&self, draft_id: &str) -> CoreResult<ScriptDraft> {
        let agent = self.draft_agent()?;
        self.drafts.retry(draft_id.trim(), agent)
    }

    /// Stop draft `draft_id`'s turn under way (it ends `cancelled`). Only
    /// the user stops a draft: nothing else does.
    pub fn cancel_script_draft(&self, draft_id: &str) -> CoreResult<ScriptDraft> {
        self.drafts.cancel(draft_id.trim())
    }

    /// Every draft, oldest first.
    pub fn list_script_drafts(&self) -> Vec<ScriptDraft> {
        self.drafts.list()
    }

    /// Forget draft `draft_id`, stopping it first when it runs.
    pub fn discard_script_draft(&self, draft_id: &str) -> CoreResult<()> {
        self.drafts.discard(draft_id.trim())
    }

    /// Mark draft `draft_id` as the draft of the submit script `script_id`
    /// (after saving it): a later save replaces that script.
    pub fn link_script_draft(&self, draft_id: &str, script_id: &str) -> CoreResult<ScriptDraft> {
        let script_id = script_id.trim();
        if !self
            .settings()
            .submit_scripts
            .iter()
            .any(|s| s.id == script_id)
        {
            return Err(CoreError::NotFound);
        }
        self.drafts.link(draft_id.trim(), script_id)
    }

    /// Register the `script-draft:state` hook: called with the whole draft
    /// on every status change (on the caller's or the draft's thread).
    pub fn on_script_draft(&self, callback: impl Fn(&ScriptDraft) + Send + Sync + 'static) {
        self.drafts.set_callback(Arc::new(callback));
    }

    /// Register the `script-draft:progress` hook: a running turn's stage,
    /// thinking and answer size, at most every [`PROGRESS_EVERY`] per
    /// draft (a stage change at once), on the draft's thread.
    pub fn on_script_draft_progress(
        &self,
        callback: impl Fn(&ScriptDraftProgressEvent) + Send + Sync + 'static,
    ) {
        self.drafts.set_progress_callback(Arc::new(callback));
    }

    /// Wait until no draft runs or waits (tests; `false` when `timeout`
    /// passed first).
    pub fn wait_script_drafts_idle(&self, timeout: Duration) -> bool {
        self.drafts.wait_idle(timeout)
    }
}

/// A request as sent: not blank, not too long.
fn message(text: &str) -> CoreResult<&str> {
    let text = text.trim();
    if text.is_empty() {
        return Err(CoreError::InvalidInput(
            "describe what the script should do".into(),
        ));
    }
    if text.chars().count() > MAX_MESSAGE {
        return Err(CoreError::InvalidInput(format!(
            "keep the request under {MAX_MESSAGE} characters"
        )));
    }
    Ok(text)
}
