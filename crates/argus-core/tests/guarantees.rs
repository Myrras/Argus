//! The spec's guarantees, end to end through the public `Core` API over a
//! scripted platform (`support::Platform`) that logs every provider call:
//!
//! - (6) nothing is written to the platform without an explicit call;
//! - (7) a review leaves the active list exactly when approved;
//! - (3) whether the review waits on the user, and why, for each status;
//! - (1, 2) an outdated thread stays fully readable: notes, then/now code
//!   and the commit that removed its line;
//! - (4, 5, 9) after new commits: re-review with the since-last-review
//!   diff; from the file list to a posted comment without leaving Argus.
//!
//! The AI pass' side of (6) and (8) needs the run executor, so it lives in
//! `src/ai/guarantees.rs`.

mod support;

use std::sync::mpsc;
use std::time::Duration;

use argus_core::history::{LineChange, TimelineEvent};
use argus_core::provider::{ReviewState, Reviewer, ReviewerState, Side};
use argus_core::{
    CoreError, LineStatus, NewComment, PostErrorKind, PostResult, ReviewStatus, RoundReason,
    VerdictLabel,
};

use support::*;

fn comment(line: u32, body: &str) -> NewComment {
    NewComment {
        path: PATH.into(),
        line,
        side: Side::New,
        start_line: None,
        body: body.into(),
        head_sha: None,
        base_sha: None,
    }
}

// ------------------------------------------------------------ (6) no write

#[test]
fn nothing_reaches_the_platform_without_an_explicit_write() {
    let p = Platform::new();
    let core = core_over(&p);
    let id = the_review(&core).id;

    // Refreshes, with the platform moving underneath.
    core.refresh();
    p.with(|w| w.push_commit("c2", 11));
    core.refresh();
    p.with(|w| w.reply(MY_THREAD, AUTHOR, "Removed it in c2."));
    core.refresh();

    // The code view: files, contents, viewed marks, commentable lines.
    let files = core.list_files(id, false).unwrap();
    assert_eq!(files.files.len(), 1);
    core.file_at(id, "c2", PATH).unwrap();
    core.set_file_viewed(id, PATH, true).unwrap();
    core.set_file_viewed(id, PATH, false).unwrap();
    core.commentable_lines(id, PATH).unwrap();

    // Thread history, computed and cached.
    let mine = thread_map(&core)[MY_THREAD].thread_id;
    core.review_histories(id).unwrap();
    core.thread_history(mine).unwrap();
    core.cached_snapshot(mine).unwrap();

    // Round bookkeeping and the since-last-review diff.
    core.mark_reviewed(id).unwrap();
    p.with(|w| w.push_commit("c3", 12));
    core.refresh();
    core.list_files(id, true).unwrap();

    // The AI pass' local state (no run: that needs an agent; see
    // src/ai/guarantees.rs). The verdict override is Argus-only.
    core.ai_state(id).unwrap();
    core.findings(id).unwrap();
    core.checklist(id).unwrap();
    core.verdicts(id).unwrap();
    core.set_verdict_override(mine, Some(VerdictLabel::Addressed))
        .unwrap();

    // Write calls that are refused before anything goes out.
    let invalid = core.post_comments(id, vec![comment(3, "  ")]).unwrap();
    assert!(matches!(&invalid[0], PostResult::Failed(e) if e.kind == PostErrorKind::InvalidInput));
    assert_eq!(
        core.reply_to_thread(mine, " ").unwrap_err().kind,
        PostErrorKind::InvalidInput
    );
    assert_eq!(
        core.approve_review(id, "c1").unwrap_err().kind,
        PostErrorKind::HeadMoved
    );

    // The background loop, nudged twice.
    let (tx, rx) = mpsc::channel();
    core.start_polling(move |r| {
        let _ = tx.send(r.errors.len());
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(0));
    core.poll_now();
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(0));
    core.stop_polling();

    assert!(p.reads() > 20, "the test should exercise the reads");
    assert_eq!(p.writes(), Vec::<&str>::new());

    // A settings change rebuilds the providers from settings (GitHub off
    // here, so no `gh` is probed): still nothing written.
    let mut s = core.settings();
    s.github.enabled = false;
    s.ai.enabled = false;
    s.poll_interval_secs = 600;
    core.update_settings(s).unwrap();
    assert_eq!(p.writes(), Vec::<&str>::new());
}

#[test]
fn each_explicit_write_is_one_platform_write() {
    let p = Platform::new();
    let core = core_over(&p);
    let id = the_review(&core).id;
    let mine = thread_map(&core)[MY_THREAD].thread_id;

    let results = core
        .post_comments(
            id,
            vec![
                comment(5, "Why five?"),
                comment(6, "reject-me"),
                comment(7, ""),
            ],
        )
        .unwrap();
    assert!(results[0].is_posted());
    assert!(matches!(&results[1], PostResult::Failed(e) if e.kind == PostErrorKind::PostRejected));
    // Invalid input never reaches the platform.
    assert!(matches!(&results[2], PostResult::Failed(e) if e.kind == PostErrorKind::InvalidInput));
    assert_eq!(p.writes(), ["post_comment", "post_comment"]);

    core.reply_to_thread(mine, "Still needed?").unwrap();
    assert_eq!(p.writes(), ["post_comment", "post_comment", "reply"]);

    let head = the_review(&core).head_sha;
    core.approve_review(id, &head).unwrap();
    assert_eq!(
        p.writes(),
        ["post_comment", "post_comment", "reply", "approve"]
    );
}

// ------------------------------------------------- (7) gone when approved

#[test]
fn a_review_leaves_the_active_list_exactly_when_approved() {
    let p = Platform::new();
    let core = core_over(&p);
    let id = the_review(&core).id;
    assert_eq!(active(&core).len(), 1);

    // Every non-approved state keeps it listed.
    core.mark_reviewed(id).unwrap();
    assert_eq!(active(&core)[0].status, ReviewStatus::WaitingOnAuthor);
    p.with(|w| w.reply(MY_THREAD, AUTHOR, "Done."));
    core.refresh();
    assert_eq!(active(&core)[0].status, ReviewStatus::NeedsReReview);

    // Someone else's approval is not the user's.
    p.with(|w| {
        w.detail.reviewers = vec![Reviewer {
            username: "dana-fixture".into(),
            state: ReviewerState::Approved,
            is_me: false,
        }]
    });
    core.refresh();
    assert_eq!(active(&core).len(), 1);

    // A refused approval (the head moved on the platform) keeps it.
    p.with(|w| w.push_commit("c2", 11));
    let stale = the_review(&core).head_sha;
    assert_eq!(
        core.approve_review(id, &stale).unwrap_err().kind,
        PostErrorKind::HeadMoved
    );
    assert_eq!(active(&core).len(), 1);
    assert!(p.writes().is_empty());

    // Approved from Argus: gone at once, kept in the store.
    core.refresh();
    let head = the_review(&core).head_sha;
    let approved = core.approve_review(id, &head).unwrap();
    assert_eq!(approved.status, ReviewStatus::Approved);
    assert!(approved.approved_at.is_some());
    assert!(active(&core).is_empty());
    assert_eq!(core.reviews().unwrap().len(), 1);
    // ...and it stays gone while the platform shows the approval.
    core.refresh();
    assert!(active(&core).is_empty());

    // The approval withdrawn on the platform: back in the list.
    p.with(|w| w.detail.approved_by_me = false);
    core.refresh();
    let back = active(&core);
    assert_eq!(back.len(), 1);
    assert_ne!(back[0].status, ReviewStatus::Approved);
    assert!(back[0].approved_at.is_none());

    // Approved on the platform itself: gone on the next refresh.
    p.with(|w| w.detail.approved_by_me = true);
    core.refresh();
    assert!(active(&core).is_empty());
}

#[test]
fn a_merge_without_approval_stays_listed_as_closed() {
    let p = Platform::new();
    let core = core_over(&p);
    p.with(|w| w.detail.state = ReviewState::Merged);
    core.refresh();
    let r = &active(&core)[0];
    assert_eq!(r.status, ReviewStatus::ClosedWithoutApproval);
    assert!(r.round_reasons.is_empty());

    // Merged after the user's approval: approved, not listed.
    let p = Platform::new();
    let core = core_over(&p);
    p.with(|w| {
        w.detail.approved_by_me = true;
        w.detail.state = ReviewState::Merged;
    });
    core.refresh();
    assert!(active(&core).is_empty());
    assert_eq!(the_review(&core).status, ReviewStatus::Approved);
}

#[test]
fn a_review_that_drops_off_the_list_is_kept_but_not_active() {
    let p = Platform::new();
    let core = core_over(&p);
    p.with(|w| w.listed = false);
    core.refresh();
    assert!(active(&core).is_empty());
    let r = the_review(&core);
    assert!(!r.listed);
    // Listed again (re-requested): back.
    p.with(|w| w.listed = true);
    core.refresh();
    assert_eq!(active(&core).len(), 1);
}

// ------------------------------------------------- (3) waiting on me, why

#[test]
fn says_whether_the_review_waits_on_the_user_and_why() {
    let p = Platform::new();
    let core = core_over(&p);
    let id = the_review(&core).id;
    let state = |core: &argus_core::Core| {
        let r = the_review(core);
        (r.status, r.round_reasons, r.round)
    };

    // Never reviewed: the first round.
    assert_eq!(state(&core), (ReviewStatus::NeedsReview, vec![], 1));
    assert!(the_review(&core).requested);

    // Reviewed, nothing since: waiting on the author, also after a refresh.
    core.mark_reviewed(id).unwrap();
    assert_eq!(state(&core), (ReviewStatus::WaitingOnAuthor, vec![], 1));
    assert!(core.refresh().changed.is_empty());
    assert_eq!(state(&core).0, ReviewStatus::WaitingOnAuthor);

    // The author answered in the user's thread.
    p.with(|w| w.reply(MY_THREAD, AUTHOR, "Fixed."));
    core.refresh();
    assert_eq!(
        state(&core),
        (
            ReviewStatus::NeedsReReview,
            vec![RoundReason::AuthorReplied],
            2
        )
    );

    // Replying from Argus moves the watermark: waiting again.
    let mine = thread_map(&core)[MY_THREAD].thread_id;
    tick();
    core.reply_to_thread(mine, "Thanks, looks right.").unwrap();
    assert_eq!(state(&core), (ReviewStatus::WaitingOnAuthor, vec![], 2));

    // A note in a thread that is not the user's does not call them back.
    p.with(|w| w.reply(GENERAL_THREAD, AUTHOR, "Also bumped the TTL."));
    core.refresh();
    assert_eq!(state(&core).0, ReviewStatus::WaitingOnAuthor);

    // New commits.
    p.with(|w| w.push_commit("c2", 11));
    core.refresh();
    assert_eq!(
        state(&core),
        (
            ReviewStatus::NeedsReReview,
            vec![RoundReason::NewCommits],
            3
        )
    );
    tick();
    core.mark_reviewed(id).unwrap();
    assert_eq!(state(&core).0, ReviewStatus::WaitingOnAuthor);

    // Re-requested: a transition from not requested to requested.
    p.with(|w| w.detail.requested_from_me = false);
    core.refresh();
    assert_eq!(state(&core).0, ReviewStatus::WaitingOnAuthor);
    tick();
    p.with(|w| w.detail.requested_from_me = true);
    core.refresh();
    assert_eq!(
        state(&core),
        (
            ReviewStatus::NeedsReReview,
            vec![RoundReason::ReRequested],
            4
        )
    );

    // Several reasons at once, in a stable order; one round.
    tick();
    core.mark_reviewed(id).unwrap();
    p.with(|w| {
        w.push_commit("c3", 12);
        w.reply(MY_THREAD, AUTHOR, "And one more.");
    });
    core.refresh();
    assert_eq!(
        state(&core),
        (
            ReviewStatus::NeedsReReview,
            vec![RoundReason::NewCommits, RoundReason::AuthorReplied],
            5
        )
    );

    // The wire names the frontend shows the "why" from.
    let json = serde_json::to_value(the_review(&core)).unwrap();
    assert_eq!(json["status"], "needsReReview");
    assert_eq!(
        json["roundReasons"],
        serde_json::json!(["newCommits", "authorReplied"])
    );
}

// --------------------------------------- (1, 2) outdated, fully readable

#[test]
fn an_outdated_thread_stays_fully_readable() {
    let p = Platform::new();
    let core = core_over(&p);
    p.with(|w| {
        w.reply(MY_THREAD, AUTHOR, "Dropped it.");
        w.push_commit("c2", 11);
    });
    core.refresh();

    // Stored with every note, flagged outdated, not hidden.
    let threads = thread_map(&core);
    let mine = &threads[MY_THREAD];
    assert!(mine.mine && mine.thread.outdated);
    let bodies: Vec<&str> = mine.thread.notes.iter().map(|n| n.body.as_str()).collect();
    assert_eq!(bodies, ["Why does line 10 stay?", "Dropped it."]);

    let h = core.thread_history(mine.thread_id).unwrap();
    assert!(h.outdated);
    assert_eq!(h.head_sha, "c2");
    assert_eq!(
        h.status,
        LineStatus::Deleted {
            path: PATH.into(),
            removed_by: Some("c2".into())
        }
    );
    assert_eq!(h.touched_by, ["c2"]);
    // Then: the code the comment was made on, the line marked.
    let then = h.then_snapshot.as_ref().expect("a then snapshot");
    assert_eq!(then.sha, "c1");
    let anchored = then.anchor_end.unwrap() - then.first_line;
    assert_eq!(then.lines[anchored as usize], "line 10");
    // The timeline: both notes, then the commit that removed the line.
    let events: Vec<String> = h
        .timeline
        .iter()
        .map(|e| match &e.event {
            TimelineEvent::Note { body, .. } => format!("note: {body}"),
            TimelineEvent::Commit { sha, change, .. } => format!("commit {sha}: {change:?}"),
            other => format!("{other:?}"),
        })
        .collect();
    assert!(events.contains(&"note: Why does line 10 stay?".to_string()));
    assert!(events.contains(&"note: Dropped it.".to_string()));
    assert!(events.contains(&format!("commit c2: {:?}", LineChange::Deleted)));

    // The review-level thread reads as notes only.
    let general = core
        .thread_history(threads[GENERAL_THREAD].thread_id)
        .unwrap();
    assert_eq!(general.status, LineStatus::NotCode);
    assert_eq!(general.timeline.len(), 1);

    // An unknown thread is an error, not an empty history.
    assert!(matches!(
        core.thread_history(argus_core::ThreadId::new()),
        Err(CoreError::NotFound)
    ));
}

// ------------------------- (4, 5, 9) re-review, then comment, in Argus

#[test]
fn from_new_commits_to_a_posted_comment_without_leaving_argus() {
    let p = Platform::new();
    let core = core_over(&p);
    let id = the_review(&core).id;
    core.mark_reviewed(id).unwrap();
    p.with(|w| w.push_commit("c2", 11));
    core.refresh();
    assert_eq!(the_review(&core).status, ReviewStatus::NeedsReReview);

    // The whole review, file by file, and only what changed since.
    let all = core.list_files(id, false).unwrap();
    assert_eq!((all.base_sha.as_str(), all.head_sha.as_str()), ("b0", "c2"));
    let since = core.list_files(id, true).unwrap();
    assert!(since.since_last_review);
    assert_eq!(
        (since.base_sha.as_str(), since.head_sha.as_str()),
        ("c1", "c2")
    );
    let patch = since.files[0].file.patch.as_deref().unwrap();
    assert!(patch.contains("-line 10") && !patch.contains("+line five"));
    assert_eq!(core.file_at(id, "c2", PATH).unwrap(), file_v("c2"));

    // Comment on a line the gutter offers.
    let lines = core.commentable_lines(id, PATH).unwrap();
    let line = lines
        .hunks
        .iter()
        .flat_map(|h| h.new.iter().copied())
        .next()
        .expect("a commentable new-side line");
    let results = core
        .post_comments(id, vec![comment(line, "Is this still needed?")])
        .unwrap();
    let PostResult::Posted(posted) = &results[0] else {
        panic!("not posted: {results:?}");
    };
    assert_eq!((posted.line, posted.head_sha.as_str()), (line, "c2"));

    // The post refreshed the review: the thread is there, the round done.
    let r = the_review(&core);
    assert_eq!(r.status, ReviewStatus::WaitingOnAuthor);
    assert_eq!(r.last_reviewed_sha.as_deref(), Some("c2"));
    assert!(thread_map(&core).contains_key(&posted.provider_thread_id));
}
