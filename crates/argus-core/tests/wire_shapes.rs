//! The wire shapes the webview sees, as a snapshot both sides check:
//! `src/ipc/wire-shapes.json` holds every key path of the main IPC structs
//! (serialised from fully filled samples here) and the string values of the
//! wire enums. `src/ipc/wire.test.ts` computes the same from literals typed
//! against `src/ipc/types.ts`, so a field renamed, added or dropped on one
//! side fails a test on the other.
//!
//! After a deliberate change of a wire type, regenerate the snapshot with
//! `ARGUS_UPDATE_WIRE=1 cargo test -p argus-core --test wire_shapes`, then
//! update `types.ts` until the Vitest side passes too.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use argus_core::ai::{
    AiRunState, AiRunStatus, ChecklistId, ChecklistItem, Finding, FindingId, FindingPostOutcome,
    FindingPostResult, RelatedRef, RepeatOf, RepeatReason, Severity, Verdict, VerdictLabel,
};
use argus_core::files::{FileEntry, ReviewFiles};
use argus_core::history::{
    CodeSnapshot, DiffCell, DiffKind, DiffRow, LineChange, LineStatus, SnapshotSource,
    ThreadHistory, TimelineEntry, TimelineEvent,
};
use argus_core::post::{
    CommentableHunk, CommentableLines, NewComment, PostError, PostErrorKind, PostResult,
    PostedComment, PostedNote,
};
use argus_core::provider::{
    Anchor, ChangedFile, FileStatus, GithubStatus, GitlabProblem, GitlabStatus, Note, ProviderId,
    ProviderStatus, ReviewRef, ReviewState, Side, Thread,
};
use argus_core::{
    MyReviewState, RefreshError, RefreshReport, ReviewId, ReviewRecord, ReviewStatus,
    ReviewVerdict, RoundReason, Settings, StoredThread, ThreadId,
};
use chrono::{TimeZone, Utc};
use serde::Serialize;
use serde_json::{json, Value};

fn snapshot_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/ipc/wire-shapes.json")
}

/// Every key path of `v`: `a`, `a.b` for a nested object, `a[].b` for an
/// array of objects (its first element). Mirrors `shape()` in wire.test.ts.
fn shape(v: &Value) -> Vec<String> {
    fn walk(v: &Value, prefix: &str, out: &mut BTreeSet<String>) {
        let Value::Object(map) = v else { return };
        for (k, child) in map {
            let path = format!("{prefix}{k}");
            out.insert(path.clone());
            match child {
                Value::Object(_) => walk(child, &format!("{path}."), out),
                Value::Array(items) => {
                    if let Some(first @ Value::Object(_)) = items.first() {
                        walk(first, &format!("{path}[]."), out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(v, "", &mut out);
    out.into_iter().collect()
}

fn val(x: impl Serialize) -> Value {
    serde_json::to_value(x).unwrap()
}

fn values<T: Serialize>(all: &[T]) -> Vec<String> {
    all.iter()
        .map(|x| match val(x) {
            Value::String(s) => s,
            other => other["kind"].as_str().unwrap().to_string(),
        })
        .collect()
}

// The variant lists below match on every variant, so a new one fails to
// compile here until it is added (and then fails the snapshot until the
// TS side has it too).

fn review_statuses() -> Vec<ReviewStatus> {
    use ReviewStatus::*;
    let all = vec![
        NeedsReview,
        NeedsReReview,
        WaitingOnAuthor,
        ClosedWithoutApproval,
        Approved,
    ];
    for s in &all {
        match s {
            NeedsReview | NeedsReReview | WaitingOnAuthor | ClosedWithoutApproval | Approved => {}
        }
    }
    all
}

fn round_reasons() -> Vec<RoundReason> {
    use RoundReason::*;
    let all = vec![NewCommits, AuthorReplied, ReRequested];
    for r in &all {
        match r {
            NewCommits | AuthorReplied | ReRequested => {}
        }
    }
    all
}

fn post_error_kinds() -> Vec<PostErrorKind> {
    use PostErrorKind::*;
    let all = vec![
        NotFound,
        InvalidInput,
        GithubUnavailable,
        GitlabNotConfigured,
        GitlabAuth,
        Provider,
        Store,
        Io,
        PostRejected,
        LineNotInDiff,
        HeadMoved,
        Unsupported,
    ];
    for k in &all {
        match k {
            NotFound | InvalidInput | GithubUnavailable | GitlabNotConfigured | GitlabAuth
            | Provider | Store | Io | PostRejected | LineNotInDiff | HeadMoved | Unsupported => {}
        }
    }
    all
}

fn line_statuses() -> Vec<LineStatus> {
    let all = vec![
        LineStatus::Located {
            path: "src/cache.rs".into(),
            start_line: 9,
            line: 10,
            moved: true,
            renamed: false,
            touched: true,
        },
        LineStatus::Deleted {
            path: "src/cache.rs".into(),
            removed_by: Some("c2".into()),
        },
        LineStatus::FileGone {
            path: "src/old.rs".into(),
            removed_by: Some("c2".into()),
        },
        LineStatus::Unlocated {
            reason: "rebased".into(),
        },
        LineStatus::NotCode,
    ];
    for s in &all {
        match s {
            LineStatus::Located { .. }
            | LineStatus::Deleted { .. }
            | LineStatus::FileGone { .. }
            | LineStatus::Unlocated { .. }
            | LineStatus::NotCode => {}
        }
    }
    all
}

fn line_changes() -> Vec<LineChange> {
    let all = vec![
        LineChange::Modified,
        LineChange::Deleted,
        LineChange::FileGone,
        LineChange::Renamed {
            from: "src/old.rs".into(),
            to: "src/new.rs".into(),
        },
    ];
    for c in &all {
        match c {
            LineChange::Modified
            | LineChange::Deleted
            | LineChange::FileGone
            | LineChange::Renamed { .. } => {}
        }
    }
    all
}

fn timeline_entries() -> Vec<TimelineEntry> {
    let at = Some(Utc.with_ymd_and_hms(2026, 10, 10, 9, 0, 0).unwrap());
    let all = vec![
        TimelineEntry {
            at,
            event: TimelineEvent::Note {
                note_id: "n1".into(),
                author: "rev-fixture".into(),
                body: "Why?".into(),
                system: false,
                root: true,
            },
        },
        TimelineEntry {
            at,
            event: TimelineEvent::Commit {
                sha: "c2".into(),
                title: "Commit c2".into(),
                author: "alice-fixture".into(),
                change: LineChange::Renamed {
                    from: "src/old.rs".into(),
                    to: "src/new.rs".into(),
                },
            },
        },
        TimelineEntry {
            at,
            event: TimelineEvent::Resolved {
                by: "alice-fixture".into(),
            },
        },
        TimelineEntry {
            at,
            event: TimelineEvent::Unresolved {
                by: "alice-fixture".into(),
            },
        },
    ];
    for e in &all {
        match e.event {
            TimelineEvent::Note { .. }
            | TimelineEvent::Commit { .. }
            | TimelineEvent::Resolved { .. }
            | TimelineEvent::Unresolved { .. } => {}
        }
    }
    all
}

/// The snapshot as the Rust types serialise today.
fn current() -> Value {
    let t = Utc.with_ymd_and_hms(2026, 10, 10, 9, 0, 0).unwrap();
    let review_id = ReviewId::from_key("github|github.com|acme-fixture/widgets|12");
    let thread_id = ThreadId::from_key(review_id, "4001");
    let review = ReviewRef {
        provider: ProviderId::Github,
        host: "github.com".into(),
        repo: "acme-fixture/widgets".into(),
        number: 12,
        url: "https://github.com/acme-fixture/widgets/pull/12".into(),
        title: "Add widget caching".into(),
        author: "alice-fixture".into(),
        updated_at: Some(t),
    };
    let record = ReviewRecord {
        id: review_id,
        review,
        state: ReviewState::Open,
        draft: false,
        head_sha: "c2".into(),
        base_sha: "b0".into(),
        last_reviewed_sha: Some("c1".into()),
        last_reviewed_at: Some(t),
        approved_at: Some(t),
        round: 2,
        round_reasons: vec![RoundReason::NewCommits],
        status: ReviewStatus::NeedsReReview,
        untouched_threads: 1,
        requested: true,
        rerequested_at: Some(t),
        my_review_state: MyReviewState::RequestedChanges,
        my_reviewed_at: Some(t),
        my_reviewed_sha: Some("c1".into()),
        listed: true,
        viewer: "rev-fixture".into(),
        fingerprint: "never on the wire".into(),
        fetched_at: t,
    };
    let note = Note {
        id: "n1".into(),
        author: "rev-fixture".into(),
        body: "Why?".into(),
        created_at: t,
        system: false,
    };
    let thread = StoredThread {
        thread_id,
        review_id,
        mine: true,
        started_by_me: true,
        thread: Thread {
            id: "4001".into(),
            author: "rev-fixture".into(),
            created_at: t,
            resolved: false,
            outdated: true,
            anchor: Some(Anchor {
                path_old: Some("src/cache.rs".into()),
                path_new: Some("src/cache.rs".into()),
                line_old: Some(9),
                line_new: Some(10),
                start_line: Some(9),
                side: Side::New,
                base_sha: Some("b0".into()),
                start_sha: Some("b0".into()),
                head_sha: "c1".into(),
                diff_hunk: Some("@@ -1 +1 @@".into()),
                current_line: Some(10),
            }),
            notes: vec![note],
        },
    };
    let files = ReviewFiles {
        review_id,
        base_sha: "b0".into(),
        head_sha: "c2".into(),
        since_last_review: false,
        truncated: false,
        base_approximate: false,
        review_base_sha: "b0".into(),
        files: vec![FileEntry {
            file: ChangedFile {
                path: "src/cache.rs".into(),
                old_path: Some("src/old.rs".into()),
                status: FileStatus::Renamed,
                additions: 1,
                deletions: 1,
                patch: Some("@@ -1 +1 @@\n-a\n+b".into()),
                content_sha: "blob".into(),
                binary: false,
                too_large: false,
            },
            viewed: true,
        }],
    };
    let commentable = CommentableLines {
        review_id,
        path: "src/cache.rs".into(),
        base_sha: "b0".into(),
        head_sha: "c2".into(),
        hunks: vec![CommentableHunk {
            old: vec![1],
            new: vec![1, 2],
        }],
    };
    let new_comment = NewComment {
        path: "src/cache.rs".into(),
        line: 10,
        side: Side::New,
        start_line: Some(9),
        body: "Why?".into(),
        head_sha: Some("c2".into()),
        base_sha: Some("b0".into()),
    };
    let posted = PostedComment {
        url: "https://github.com/acme-fixture/widgets/pull/12#discussion_r1".into(),
        provider_thread_id: "1".into(),
        provider_note_id: "1".into(),
        path: "src/cache.rs".into(),
        side: Side::New,
        line: 10,
        start_line: Some(9),
        head_sha: "c2".into(),
    };
    let post_error = PostError::new(PostErrorKind::PostRejected, "GitHub: refused");
    let report = RefreshReport {
        changed: vec![review_id],
        errors: vec![RefreshError {
            provider: ProviderId::Gitlab,
            review: Some(review_id),
            message: "GitLab is not set up.".into(),
            unavailable: true,
            rate_limited_until: Some(t),
        }],
        refreshed_at: t,
    };
    let mut settings = Settings::default();
    settings.github.account = Some("work".into());
    settings.ai.binary = Some("/usr/local/bin/claude".into());
    let status = ProviderStatus {
        github: GithubStatus {
            enabled: true,
            host: "github.com".into(),
            installed: true,
            logged_in: true,
        },
        gitlab: GitlabStatus {
            configured: true,
            base_url: "https://gitlab.example.test".into(),
            token_present: true,
            problem: Some(GitlabProblem::InvalidUrl),
            insecure: true,
        },
    };
    let snapshot = CodeSnapshot {
        sha: "c1".into(),
        path: "src/cache.rs".into(),
        first_line: 1,
        lines: vec!["line 1".into()],
        anchor_start: Some(1),
        anchor_end: Some(1),
        source: SnapshotSource::File,
    };
    let cell = DiffCell {
        line: 1,
        text: "line 1".into(),
        anchored: true,
    };
    let history = ThreadHistory {
        thread_id,
        review_id,
        head_sha: "c2".into(),
        outdated: true,
        status: line_statuses().remove(0),
        touched_by: vec!["c2".into()],
        then_snapshot: Some(snapshot.clone()),
        now_snapshot: Some(snapshot),
        diff_lines: vec![DiffRow {
            kind: DiffKind::Changed,
            then: Some(cell.clone()),
            now: Some(cell),
        }],
        timeline: vec![timeline_entries().remove(0)],
    };
    let ai_state = AiRunState {
        review_id,
        enabled: true,
        status: AiRunStatus::Done,
        queued: false,
        round: Some(2),
        adapter: Some("claude".into()),
        error: Some("cancelled".into()),
        started_at: Some(t),
        finished_at: Some(t),
        last_done_round: Some(2),
        dropped_findings: 0,
        dropped_verdicts: 0,
        dropped_duplicates: 0,
        skipped_checklist: 0,
        read_only_guaranteed: true,
    };
    let finding_id = FindingId::new();
    let finding = Finding {
        id: finding_id,
        review_id,
        round: 2,
        path: "src/cache.rs".into(),
        line: 10,
        side: Side::New,
        severity: Severity::High,
        title: "TTL of zero".into(),
        body: "Every lookup misses.".into(),
        dismissed: false,
        posted_at: Some(t),
        posted_url: Some("https://github.com/acme-fixture/widgets/pull/12#r2".into()),
        repeat_of: Some(RepeatOf {
            reason: RepeatReason::AgentMarked,
            thread_id: Some(thread_id),
            finding_id: Some(finding_id),
        }),
        related_to: vec![RelatedRef {
            thread_id: Some(thread_id),
            finding_id: Some(finding_id),
        }],
    };
    let checklist = ChecklistItem {
        id: ChecklistId::new(),
        review_id,
        round: 2,
        text: "Is this intended?".into(),
        checked: false,
        repeat_of: Some(RepeatOf {
            reason: RepeatReason::Similar,
            thread_id: Some(thread_id),
            finding_id: Some(finding_id),
        }),
        related_to: vec![RelatedRef {
            thread_id: Some(thread_id),
            finding_id: Some(finding_id),
        }],
    };
    let verdict = Verdict {
        thread_id,
        review_id,
        round: 2,
        ai_label: Some(VerdictLabel::Partly),
        ai_reason: Some("Half of it.".into()),
        user_override: Some(VerdictLabel::Addressed),
        label: Some(VerdictLabel::Addressed),
    };

    let mut structs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut add = |name: &str, v: Value| {
        structs.insert(name.to_string(), shape(&v));
    };
    add("ReviewRecord", val(&record));
    add("StoredThread", val(&thread));
    add("ReviewFiles", val(&files));
    add("CommentableLines", val(&commentable));
    add("NewComment", val(&new_comment));
    add("PostResult:posted", val(PostResult::Posted(posted)));
    add(
        "PostResult:failed",
        val(PostResult::Failed(post_error.clone())),
    );
    add(
        "PostedNote",
        val(PostedNote {
            url: "u".into(),
            provider_thread_id: "1".into(),
            provider_note_id: "2".into(),
        }),
    );
    add("RefreshReport", val(&report));
    add("Settings", val(&settings));
    add("ProviderStatus", val(&status));
    add("ThreadHistory", val(&history));
    // One sample per tagged variant: `<Name>:<kind>`.
    for v in line_statuses().iter().map(val) {
        add(
            &format!("LineStatus:{}", v["kind"].as_str().unwrap()),
            v.clone(),
        );
    }
    for v in timeline_entries().iter().map(val) {
        add(
            &format!("TimelineEntry:{}", v["kind"].as_str().unwrap()),
            v.clone(),
        );
    }
    add("AiRunState", val(&ai_state));
    add("Finding", val(&finding));
    add("ChecklistItem", val(&checklist));
    add("Verdict", val(&verdict));
    add(
        "FindingPostResult:posted",
        val(FindingPostResult {
            finding_id,
            outcome: FindingPostOutcome::Posted {
                posted_url: "u".into(),
                posted_at: t,
            },
        }),
    );
    add(
        "FindingPostResult:failed",
        val(FindingPostResult {
            finding_id,
            outcome: FindingPostOutcome::Failed(post_error),
        }),
    );

    let enums: BTreeMap<&str, Vec<String>> = BTreeMap::from([
        (
            "ProviderId",
            values(&[ProviderId::Github, ProviderId::Gitlab]),
        ),
        (
            "ReviewState",
            values(&[ReviewState::Open, ReviewState::Merged, ReviewState::Closed]),
        ),
        ("ReviewStatus", values(&review_statuses())),
        (
            "MyReviewState",
            values(&[
                MyReviewState::AwaitingReview,
                MyReviewState::ReviewStarted,
                MyReviewState::RequestedChanges,
                MyReviewState::Commented,
                MyReviewState::Approved,
                MyReviewState::NotReviewer,
            ]),
        ),
        (
            "ReviewVerdict",
            values(&[
                ReviewVerdict::Approve,
                ReviewVerdict::RequestChanges,
                ReviewVerdict::Comment,
            ]),
        ),
        ("RoundReason", values(&round_reasons())),
        ("Side", values(&[Side::Old, Side::New])),
        ("GitlabProblem", values(&[GitlabProblem::InvalidUrl])),
        (
            "FileStatus",
            values(&[
                FileStatus::Added,
                FileStatus::Modified,
                FileStatus::Removed,
                FileStatus::Renamed,
            ]),
        ),
        ("PostErrorKind", values(&post_error_kinds())),
        ("LineStatus.kind", values(&line_statuses())),
        ("LineChange.kind", values(&line_changes())),
        (
            "TimelineEvent.kind",
            values(
                &timeline_entries()
                    .into_iter()
                    .map(|e| e.event)
                    .collect::<Vec<_>>(),
            ),
        ),
        (
            "SnapshotSource",
            values(&[SnapshotSource::File, SnapshotSource::DiffHunk]),
        ),
        (
            "DiffKind",
            values(&[
                DiffKind::Same,
                DiffKind::Changed,
                DiffKind::Removed,
                DiffKind::Added,
            ]),
        ),
        (
            "AiRunStatus",
            values(&[
                AiRunStatus::NotStarted,
                AiRunStatus::Running,
                AiRunStatus::Done,
                AiRunStatus::Failed,
            ]),
        ),
        (
            "Severity",
            values(&[Severity::High, Severity::Medium, Severity::Low]),
        ),
        (
            "VerdictLabel",
            values(&[
                VerdictLabel::Addressed,
                VerdictLabel::Partly,
                VerdictLabel::Untouched,
            ]),
        ),
        (
            "RepeatReason",
            values(&[RepeatReason::AgentMarked, RepeatReason::Similar]),
        ),
    ]);
    json!({ "structs": structs, "enums": enums })
}

#[test]
fn the_wire_shapes_match_the_snapshot() {
    let now = current();
    let path = snapshot_path();
    let text = serde_json::to_string_pretty(&now).unwrap() + "\n";
    if std::env::var_os("ARGUS_UPDATE_WIRE").is_some() {
        std::fs::write(&path, text).unwrap();
        return;
    }
    let stored: Value = serde_json::from_str(
        &std::fs::read_to_string(&path).expect("src/ipc/wire-shapes.json exists"),
    )
    .unwrap();
    assert!(
        stored == now,
        "the Rust wire types no longer match src/ipc/wire-shapes.json. After a \
         deliberate change, run `ARGUS_UPDATE_WIRE=1 cargo test -p argus-core --test \
         wire_shapes` and update src/ipc/types.ts.\n\nnow:\n{text}"
    );
}

#[test]
fn fingerprint_and_secrets_never_go_on_the_wire() {
    let shapes = current();
    let record = shapes["structs"]["ReviewRecord"].as_array().unwrap();
    assert!(!record.iter().any(|k| k == "fingerprint"));
    let settings = shapes["structs"]["Settings"].as_array().unwrap();
    assert!(!settings.iter().any(|k| {
        let k = k.as_str().unwrap().to_ascii_lowercase();
        k.contains("token") || k.contains("secret") || k.contains("password")
    }));
}
