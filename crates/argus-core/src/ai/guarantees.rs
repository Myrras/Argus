//! The spec's guarantees where the AI pass is involved, end to end through
//! [`Core`] over a provider that logs every call (the rest are in
//! `tests/guarantees.rs`):
//!
//! - (6) an AI run, queued by a refresh or started by hand, only reads
//!   from the platform; a finding reaches it only through `post_findings`;
//! - (4) a later round's run covers the new commits and gives a verdict on
//!   the user's open thread;
//! - (8) with the AI pass off, tracking and thread history work as before
//!   and no run ever starts.
//!
//! The run is the real [`job::execute`] against a fake `claude` script;
//! only the agent binary is swapped in.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;

use super::job::{self, JobContext};
use super::model::{AiRunStatus, FindingPost, FindingPostOutcome, VerdictLabel};
use super::output::RunOutput;
use super::runtime::Executor;
use crate::error::{CoreError, CoreResult};
use crate::ids::ThreadId;
use crate::post::{PlacedComment, PostError, PostedComment, PostedNote};
use crate::provider::{
    Anchor, ChangedFile, CommitInfo, DiffRange, FileDiff, FileStatus, Note, ReviewDetail,
    ReviewProvider, ReviewRef, Side, Thread,
};
use crate::review::{ReviewStatus, RoundReason};
use crate::settings::Settings;
use crate::store::{ReviewRecord, Store};
use crate::testutil::{at, detail, review_ref, AUTHOR, ME};
use crate::{Core, LineStatus, RefreshReport};

const PATH: &str = "src/cache.rs";
const MY_THREAD: &str = "4001";

/// `src/cache.rs` at `sha`: `line 1`..`line 30`; `c1` rewrites line 5, `c2`
/// also deletes line 10.
fn file_v(sha: &str) -> String {
    let mut lines: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
    if sha != "b0" {
        lines[4] = "line five".into();
    }
    if sha == "c2" {
        lines.remove(9);
    }
    lines.iter().map(|l| format!("{l}\n")).collect()
}

fn diff(from: &str, to: &str) -> FileDiff {
    let patch = similar::TextDiff::from_lines(&file_v(from), &file_v(to))
        .unified_diff()
        .context_radius(2)
        .to_string();
    FileDiff {
        base_sha: from.into(),
        head_sha: to.into(),
        files: vec![ChangedFile {
            path: PATH.into(),
            old_path: None,
            status: FileStatus::Modified,
            additions: 1,
            deletions: 1,
            patch: Some(patch.trim_end().to_string()),
            content_sha: format!("blob-{to}"),
            binary: false,
            too_large: false,
        }],
        truncated: false,
        base_approximate: false,
    }
}

fn commit(sha: &str, day: u32) -> CommitInfo {
    CommitInfo {
        sha: sha.into(),
        title: format!("Commit {sha}"),
        author: AUTHOR.into(),
        authored_at: Some(at(day, 9)),
    }
}

/// One review on a scripted platform; every call logged as
/// `(method, writes)`.
struct Recorder {
    detail: Mutex<ReviewDetail>,
    threads: Mutex<Vec<Thread>>,
    calls: Mutex<Vec<(&'static str, bool)>>,
}

fn said(id: &str, author: &str, body: &str, day: u32, hour: u32) -> Note {
    Note {
        id: id.into(),
        author: author.into(),
        body: body.into(),
        created_at: at(day, hour),
        system: false,
    }
}

impl Recorder {
    /// The user's own thread on line 10.
    fn new() -> Arc<Self> {
        Self::with_thread(vec![said(MY_THREAD, ME, "Why does line 10 stay?", 10, 11)])
    }

    /// One thread on line 10 with `notes` (the first is the root).
    fn with_thread(notes: Vec<Note>) -> Arc<Self> {
        let mut d = detail("c1");
        d.base_sha = "b0".into();
        d.commits = vec![commit("c1", 10)];
        let mine = Thread {
            id: MY_THREAD.into(),
            author: notes[0].author.clone(),
            created_at: notes[0].created_at,
            resolved: false,
            resolvable: true,
            outdated: false,
            anchor: Some(Anchor {
                path_old: Some(PATH.into()),
                path_new: Some(PATH.into()),
                line_old: None,
                line_new: Some(10),
                start_line: None,
                side: Side::New,
                base_sha: None,
                start_sha: None,
                head_sha: "c1".into(),
                diff_hunk: None,
                current_line: None,
            }),
            notes,
        };
        Arc::new(Self {
            detail: Mutex::new(d),
            threads: Mutex::new(vec![mine]),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn log(&self, method: &'static str, write: bool) {
        self.calls.lock().unwrap().push((method, write));
    }

    fn writes(&self) -> Vec<&'static str> {
        let calls = self.calls.lock().unwrap();
        calls.iter().filter(|(_, w)| *w).map(|(m, _)| *m).collect()
    }

    fn read(&self, method: &str) -> usize {
        let calls = self.calls.lock().unwrap();
        calls.iter().filter(|(m, w)| *m == method && !w).count()
    }

    /// `c2` lands (deleting line 10) and the author answers.
    fn push_c2_and_reply(&self) {
        let mut d = self.detail.lock().unwrap();
        d.head_sha = "c2".into();
        d.commits.push(commit("c2", 11));
        let mut threads = self.threads.lock().unwrap();
        threads[0].outdated = true;
        let start = Utc::now();
        while Utc::now() <= start {
            std::hint::spin_loop();
        }
        threads[0].notes.push(Note {
            id: "4003".into(),
            author: AUTHOR.into(),
            body: "Removed it.".into(),
            created_at: Utc::now(),
            system: false,
        });
    }
}

impl ReviewProvider for Recorder {
    fn id(&self) -> crate::provider::ProviderId {
        crate::provider::ProviderId::Github
    }
    fn host(&self) -> &str {
        "github.com"
    }
    fn current_user(&self) -> CoreResult<String> {
        self.log("current_user", false);
        Ok(ME.into())
    }
    fn list_reviews(&self) -> CoreResult<Vec<ReviewRef>> {
        self.log("list_reviews", false);
        Ok(vec![review_ref(12)])
    }
    fn fetch_review(&self, _: &ReviewRef) -> CoreResult<ReviewDetail> {
        self.log("fetch_review", false);
        Ok(self.detail.lock().unwrap().clone())
    }
    fn fetch_threads(&self, _: &ReviewRef) -> CoreResult<Vec<Thread>> {
        self.log("fetch_threads", false);
        Ok(self.threads.lock().unwrap().clone())
    }
    fn file_at(&self, _: &ReviewRef, sha: &str, path: &str) -> CoreResult<String> {
        self.log("file_at", false);
        (path == PATH)
            .then(|| file_v(sha))
            .ok_or(CoreError::NotFound)
    }
    fn fetch_files(&self, _: &ReviewRef, range: &DiffRange) -> CoreResult<FileDiff> {
        self.log("fetch_files", false);
        Ok(match range {
            DiffRange::Review { base, head } => diff(base, head),
            DiffRange::Compare { from, to } => diff(from, to),
        })
    }
    fn commit_diff(&self, _: &ReviewRef, sha: &str) -> CoreResult<FileDiff> {
        self.log("commit_diff", false);
        match sha {
            "c1" => Ok(diff("b0", "c1")),
            "c2" => Ok(diff("c1", "c2")),
            _ => Err(CoreError::NotFound),
        }
    }
    fn post_comment(&self, r: &ReviewRef, c: &PlacedComment) -> Result<PostedComment, PostError> {
        self.log("post_comment", true);
        Ok(PostedComment {
            url: format!("{}#discussion_r5001", r.url),
            provider_thread_id: "5001".into(),
            provider_note_id: "5001".into(),
            path: c.path.clone(),
            side: c.side,
            line: c.line_number(),
            start_line: c.start_number(),
            head_sha: c.head_sha.clone(),
        })
    }
    fn reply(&self, _: &ReviewRef, _: &str, _: &str) -> Result<PostedNote, PostError> {
        self.log("reply", true);
        Err(PostError::new(
            crate::post::PostErrorKind::PostRejected,
            "not in these tests",
        ))
    }
    fn approve(&self, _: &ReviewRef, _: &str) -> Result<(), PostError> {
        self.log("approve", true);
        Ok(())
    }
    // Logged as a write like the others, so a run that resolved a thread
    // would fail the "only reads" checks.
    fn set_thread_resolved(&self, _: &ReviewRef, _: &str, _: bool) -> Result<(), PostError> {
        self.log("set_thread_resolved", true);
        Ok(())
    }
}

fn report() -> RefreshReport {
    RefreshReport {
        changed: Vec::new(),
        errors: Vec::new(),
        refreshed_at: Utc::now(),
    }
}

fn core_over(recorder: &Arc<Recorder>, settings: Settings) -> Core {
    let provider: Arc<dyn ReviewProvider> = recorder.clone();
    let core = Core::with_parts(
        Store::in_memory().unwrap(),
        None,
        settings,
        Some(vec![provider]),
    );
    assert!(core.refresh().errors.is_empty());
    core
}

fn the_review(core: &Core) -> ReviewRecord {
    core.reviews().unwrap().remove(0)
}

/// A fake `claude` that prints `answer.json` from its own dir as the
/// stream's result and counts its runs.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
dir="$(dirname "$0")"
n=$(cat "$dir/count" 2>/dev/null || echo 0)
echo $((n + 1)) > "$dir/count"
cat > "$dir/stdin.txt"
printf '{"type":"result","subtype":"success","is_error":false,"result":%s}\n' "$(cat "$dir/answer.json")"
"#;

struct Agent {
    dir: tempfile::TempDir,
    settings: Settings,
}

impl Agent {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("claude");
        crate::testutil::write_executable(&script, FAKE_CLAUDE);
        let mut settings = Settings::default();
        settings.github.enabled = false;
        settings.ai.binary = Some(script.to_string_lossy().into_owned());
        Self { dir, settings }
    }

    /// The agent's next answer (a JSON string literal holding the JSON).
    fn answer(&self, value: serde_json::Value) {
        let text = serde_json::to_string(&value.to_string()).unwrap();
        std::fs::write(self.dir.path().join("answer.json"), text).unwrap();
    }

    fn runs(&self) -> u32 {
        std::fs::read_to_string(self.dir.path().join("count"))
            .map(|s| s.trim().parse().unwrap())
            .unwrap_or(0)
    }

    fn prompt(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("stdin.txt")).unwrap()
    }

    /// The real job with this agent: whatever settings the run was queued
    /// with, the binary is the fake.
    fn executor(&self) -> Executor {
        let settings = self.settings.clone();
        Arc::new(
            move |store: &Store, rec: &ReviewRecord, ctx: &JobContext, cancel: &AtomicBool| {
                let mut ctx = ctx.clone();
                ctx.settings = settings.clone();
                job::execute(store, rec, &ctx, cancel)
            },
        )
    }
}

#[test]
#[cfg_attr(windows, ignore)]
fn ai_runs_only_read_and_findings_post_only_when_asked() {
    let recorder = Recorder::new();
    let agent = Agent::new();
    let core = core_over(&recorder, agent.settings.clone());
    core.ai.set_executor(agent.executor());
    let id = the_review(&core).id;
    let mine = ThreadId::from_key(id, MY_THREAD);

    // Round 1, queued by the refresh hook.
    agent.answer(serde_json::json!({
        "findings": [{"path": PATH, "line": 5, "side": "new", "severity": "high",
                      "title": "Five is spelled out", "body": "Use a number."}],
        "checklist": [{"text": "Is the rename intended?"}],
        "verdicts": []
    }));
    assert_eq!(core.on_refresh_report(&report()), vec![id]);
    assert!(core.ai.wait_idle(Duration::from_secs(20)));
    let st = core.ai_state(id).unwrap();
    assert_eq!(st.status, AiRunStatus::Done, "{:?}", st.error);
    assert_eq!(core.findings(id).unwrap().len(), 1);
    assert!(recorder.read("fetch_files") > 0);
    assert_eq!(recorder.writes(), Vec::<&str>::new());

    // Round 2: new commits and a reply in the user's thread. The run sees
    // only what changed since the round was reviewed, computes the
    // thread's history (reads) and gives it a verdict.
    core.mark_reviewed(id).unwrap();
    recorder.push_c2_and_reply();
    core.refresh();
    let r = the_review(&core);
    assert_eq!(r.status, ReviewStatus::NeedsReReview);
    assert_eq!(
        r.round_reasons,
        [RoundReason::NewCommits, RoundReason::AuthorReplied]
    );
    agent.answer(serde_json::json!({
        "findings": [],
        "checklist": [],
        "verdicts": [{"threadId": mine.to_string(), "label": "addressed",
                      "reason": "Line 10 is gone in c2."}]
    }));
    assert_eq!(core.on_refresh_report(&report()), vec![id]);
    assert!(core.ai.wait_idle(Duration::from_secs(20)));
    assert_eq!(core.ai_state(id).unwrap().last_done_round, Some(2));
    let prompt = agent.prompt();
    assert!(prompt.contains("-line 10"), "the new commit's diff");
    assert!(!prompt.contains("+line five"), "not the reviewed part");
    let verdicts = core.verdicts(id).unwrap();
    assert_eq!(verdicts.len(), 1);
    assert_eq!(verdicts[0].thread_id, mine);
    assert_eq!(verdicts[0].ai_label, Some(VerdictLabel::Addressed));
    assert!(recorder.read("commit_diff") > 0, "the history was computed");

    // A manual run, cancelling, the local edits: still reads only.
    core.run_ai(id).unwrap();
    assert!(core.ai.wait_idle(Duration::from_secs(20)));
    core.cancel_ai(id).unwrap();
    let f = core.findings(id).unwrap().remove(0);
    core.dismiss_finding(f.id, true).unwrap();
    core.dismiss_finding(f.id, false).unwrap();
    let c = core.checklist(id).unwrap().remove(0);
    core.set_checklist_checked(c.id, true).unwrap();
    core.set_verdict_override(mine, Some(VerdictLabel::Partly))
        .unwrap();
    assert_eq!(agent.runs(), 3);
    assert_eq!(recorder.writes(), Vec::<&str>::new());

    // Posting the finding is the one write.
    let results = core
        .post_findings(
            id,
            vec![FindingPost {
                finding_id: f.id,
                body: None,
            }],
        )
        .unwrap();
    assert!(matches!(
        results[0].outcome,
        FindingPostOutcome::Posted { .. }
    ));
    assert_eq!(recorder.writes(), ["post_comment"]);
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_thread_the_user_replied_in_is_judged_in_the_next_round() {
    // A colleague's thread the user answered: theirs for the AI pass too.
    let recorder = Recorder::with_thread(vec![
        said(MY_THREAD, "dana-fixture", "Line 10 looks unused.", 10, 10),
        said("4002", ME, "Agreed, it should go.", 10, 11),
    ]);
    let agent = Agent::new();
    let core = core_over(&recorder, agent.settings.clone());
    core.ai.set_executor(agent.executor());
    let id = the_review(&core).id;
    let replied = ThreadId::from_key(id, MY_THREAD);
    let stored = core.threads(id).unwrap();
    assert!(stored[0].mine && !stored[0].started_by_me);

    agent.answer(serde_json::json!({"findings": [], "checklist": [], "verdicts": []}));
    assert_eq!(core.on_refresh_report(&report()), vec![id]);
    assert!(core.ai.wait_idle(Duration::from_secs(20)));

    // The author answers in it after the user's reply: a reason for a round.
    core.mark_reviewed(id).unwrap();
    recorder.push_c2_and_reply();
    core.refresh();
    assert_eq!(
        the_review(&core).round_reasons,
        [RoundReason::NewCommits, RoundReason::AuthorReplied]
    );
    agent.answer(serde_json::json!({
        "findings": [],
        "checklist": [],
        "verdicts": [{"threadId": replied.to_string(), "label": "addressed",
                      "reason": "Line 10 is gone in c2."}]
    }));
    assert_eq!(core.on_refresh_report(&report()), vec![id]);
    assert!(core.ai.wait_idle(Duration::from_secs(20)));
    let st = core.ai_state(id).unwrap();
    assert_eq!(st.last_done_round, Some(2), "{:?}", st.error);
    let prompt = agent.prompt();
    let (_, open) = prompt.split_once("# Your open threads").unwrap();
    assert!(open.contains(&format!("threadId: {replied}")));
    assert!(open.contains("Agreed, it should go."));
    let verdicts = core.verdicts(id).unwrap();
    assert_eq!(verdicts.len(), 1);
    assert_eq!(verdicts[0].thread_id, replied);
    assert_eq!(verdicts[0].ai_label, Some(VerdictLabel::Addressed));

    let v = core
        .set_verdict_override(replied, Some(VerdictLabel::Partly))
        .unwrap();
    assert_eq!(v.label, Some(VerdictLabel::Partly));
    assert_eq!(recorder.writes(), Vec::<&str>::new());
}

#[test]
fn with_the_ai_pass_off_tracking_and_history_still_work() {
    let recorder = Recorder::new();
    let mut settings = Settings::default();
    settings.github.enabled = false;
    settings.ai.enabled = false;
    let core = core_over(&recorder, settings);
    let starts = Arc::new(AtomicUsize::new(0));
    let s2 = starts.clone();
    core.ai.set_executor(Arc::new(
        move |_: &Store, _: &ReviewRecord, _: &JobContext, _: &AtomicBool| {
            s2.fetch_add(1, Ordering::SeqCst);
            Ok(RunOutput::default())
        },
    ));
    let id = the_review(&core).id;
    let mine = ThreadId::from_key(id, MY_THREAD);

    // Tracking: first round, reviewed, back with the reasons.
    assert_eq!(the_review(&core).status, ReviewStatus::NeedsReview);
    assert!(core.on_refresh_report(&report()).is_empty());
    assert_eq!(
        core.mark_reviewed(id).unwrap().status,
        ReviewStatus::WaitingOnAuthor
    );
    recorder.push_c2_and_reply();
    let refreshed = core.refresh();
    assert_eq!(refreshed.changed, vec![id]);
    assert!(core.on_refresh_report(&refreshed).is_empty());
    let r = the_review(&core);
    assert_eq!(r.status, ReviewStatus::NeedsReReview);
    assert_eq!(
        r.round_reasons,
        [RoundReason::NewCommits, RoundReason::AuthorReplied]
    );
    assert!(r.is_active());

    // History: the outdated thread with its then/now and timeline.
    let h = core.thread_history(mine).unwrap();
    assert!(h.outdated);
    assert_eq!(
        h.status,
        LineStatus::Deleted {
            path: PATH.into(),
            removed_by: Some("c2".into())
        }
    );
    assert!(h.then_snapshot.is_some());
    assert_eq!(h.timeline.len(), 3, "two notes and the commit");
    assert!(core.cached_snapshot(mine).unwrap().is_some());

    // The code view and posting still work.
    assert_eq!(core.list_files(id, true).unwrap().base_sha, "c1");
    core.commentable_lines(id, PATH).unwrap();

    // The AI pass itself: off, empty, refuses to run.
    assert!(matches!(core.run_ai(id), Err(CoreError::InvalidInput(_))));
    let st = core.ai_state(id).unwrap();
    assert!(!st.enabled);
    assert_eq!(st.status, AiRunStatus::NotStarted);
    assert!(core.findings(id).unwrap().is_empty());
    assert!(core.checklist(id).unwrap().is_empty());
    assert!(core.verdicts(id).unwrap().is_empty());
    assert_eq!(starts.load(Ordering::SeqCst), 0);
    assert_eq!(recorder.writes(), Vec::<&str>::new());
}

/// A finding that repeats a colleague's thread is set aside, not shown and
/// never posted; showing it anyway is local, and only then can it be
/// posted (the one write).
#[test]
#[cfg_attr(windows, ignore)]
fn a_finding_set_aside_as_a_repeat_writes_nothing_until_shown_and_posted() {
    let recorder = Recorder::with_thread(vec![said(
        "n1",
        AUTHOR,
        "The widget cache keeps stale entries after a reload here.",
        10,
        11,
    )]);
    let agent = Agent::new();
    let core = core_over(&recorder, agent.settings.clone());
    core.ai.set_executor(agent.executor());
    let id = the_review(&core).id;
    let theirs = ThreadId::from_key(id, MY_THREAD);
    agent.answer(serde_json::json!({
        "findings": [
            // On a line of the diff (postable), marked as the thread's
            // (medium: a high one is never set aside on the mark alone).
            {"path": PATH, "line": 5, "side": "new", "severity": "medium",
             "title": "Stale widget cache entries survive a reload",
             "body": "After a reload the cache keeps stale widget entries.",
             "relatedThreadIds": [theirs.to_string()]},
            {"path": PATH, "line": 4, "side": "new", "severity": "low",
             "title": "Five is spelled out", "body": "Use a number.",
             "relatedThreadIds": []}
        ],
        "checklist": [
            {"text": "Does the widget cache keep stale entries after a reload?",
             "relatedThreadIds": [theirs.to_string()]},
            {"text": "Is the rename intended?"}
        ],
        "verdicts": []
    }));
    assert_eq!(core.on_refresh_report(&report()), vec![id]);
    assert!(core.ai.wait_idle(Duration::from_secs(20)));
    let st = core.ai_state(id).unwrap();
    assert_eq!(st.status, AiRunStatus::Done, "{:?}", st.error);
    assert_eq!((st.dropped_duplicates, st.skipped_checklist), (1, 1));
    let shown: Vec<_> = core
        .findings(id)
        .unwrap()
        .into_iter()
        .map(|f| f.title)
        .collect();
    assert_eq!(shown, ["Five is spelled out"]);
    let skipped = core.skipped_findings(id).unwrap();
    assert_eq!(skipped.len(), 1);
    let repeat = skipped[0].repeat_of.clone().unwrap();
    assert_eq!(repeat.thread_id, Some(theirs));
    assert_eq!(repeat.reason, super::model::RepeatReason::AgentMarked);
    let items: Vec<_> = core
        .checklist(id)
        .unwrap()
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert_eq!(items, ["Is the rename intended?"]);
    // The repeated checklist item: listed apart, shown anyway locally.
    let skipped_items = core.skipped_checklist(id).unwrap();
    assert_eq!(skipped_items.len(), 1);
    assert_eq!(
        skipped_items[0]
            .repeat_of
            .as_ref()
            .and_then(|r| r.thread_id),
        Some(theirs)
    );
    let item = core
        .show_checklist_item_anyway(skipped_items[0].id)
        .unwrap();
    assert!(item.repeat_of.is_some());
    assert_eq!(core.checklist(id).unwrap().len(), 2);
    assert!(core.skipped_checklist(id).unwrap().is_empty());

    // Set aside: not postable.
    let post = |core: &Core| {
        core.post_findings(
            id,
            vec![FindingPost {
                finding_id: skipped[0].id,
                body: None,
            }],
        )
        .unwrap()
        .remove(0)
        .outcome
    };
    assert!(matches!(post(&core), FindingPostOutcome::Failed(_)));
    assert_eq!(recorder.writes(), Vec::<&str>::new());

    // Shown anyway: a normal finding, still nothing written.
    let f = core.show_finding_anyway(skipped[0].id).unwrap();
    assert_eq!(f.repeat_of, Some(repeat));
    assert!(core.skipped_findings(id).unwrap().is_empty());
    assert_eq!(core.findings(id).unwrap().len(), 2);
    assert_eq!(recorder.writes(), Vec::<&str>::new());
    // A rerun of the round keeps it shown.
    core.run_ai(id).unwrap();
    assert!(core.ai.wait_idle(Duration::from_secs(20)));
    assert_eq!(core.findings(id).unwrap().len(), 2);
    assert_eq!(core.checklist(id).unwrap().len(), 2);
    assert_eq!(recorder.writes(), Vec::<&str>::new());

    // Posting it is the one write.
    assert!(
        matches!(post(&core), FindingPostOutcome::Failed(_)),
        "old id"
    );
    let again = core
        .findings(id)
        .unwrap()
        .into_iter()
        .find(|f| f.title.starts_with("Stale"))
        .unwrap();
    let results = core
        .post_findings(
            id,
            vec![FindingPost {
                finding_id: again.id,
                body: None,
            }],
        )
        .unwrap();
    assert!(matches!(
        results[0].outcome,
        FindingPostOutcome::Posted { .. }
    ));
    assert_eq!(recorder.writes(), ["post_comment"]);
}
