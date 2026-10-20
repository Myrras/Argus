use super::*;
use crate::provider::MyReviewState;
use crate::testutil::{at, detail, note, thread, AUTHOR, ME};

const OTHER: &str = "dana-fixture";

/// Fetch → record, from `prev`.
fn step(
    prev: Option<&ReviewRecord>,
    d: &ReviewDetail,
    threads: &[Thread],
    now: DateTime<Utc>,
) -> ReviewRecord {
    advance(prev, d, threads, ME, true, now)
}

/// Fetch → record, from `prev` stored with `stored` threads.
fn step_over(
    prev: &ReviewRecord,
    stored: &[Thread],
    d: &ReviewDetail,
    threads: &[Thread],
    now: DateTime<Utc>,
) -> ReviewRecord {
    advance_with(Some((prev, stored)), d, threads, ME, true, now, None)
}

/// A review the user marked reviewed at head `h1` on day 10, 12:00.
fn reviewed() -> (ReviewRecord, ReviewDetail) {
    let d = detail("h1");
    let first = step(None, &d, &[], at(10, 10));
    (mark_reviewed(&first, &[], at(10, 12)), d)
}

#[test]
fn first_sight_needs_review() {
    let r = step(None, &detail("h1"), &[], at(10, 10));
    assert_eq!(r.status, ReviewStatus::NeedsReview);
    assert!(r.round_reasons.is_empty());
    assert_eq!(r.round, 1);
    assert!(r.is_active());
    assert_eq!(r.last_reviewed_sha, None);
}

#[test]
fn mark_reviewed_moves_the_watermark_and_waits_on_the_author() {
    let (r, d) = reviewed();
    assert_eq!(r.last_reviewed_sha.as_deref(), Some("h1"));
    // Dated by what the user saw: the fetch at 10:10, not the click.
    assert_eq!(r.last_reviewed_at, Some(at(10, 10)));
    assert_eq!(r.status, ReviewStatus::WaitingOnAuthor);
    // Still waiting after a refresh that brings nothing new, even though
    // the platform still lists the user as requested.
    let again = step(Some(&r), &d, &[], at(10, 15));
    assert_eq!(again.status, ReviewStatus::WaitingOnAuthor);
    assert_eq!(again.round, 1);
}

#[test]
fn new_commits_start_a_new_round() {
    let (r, _) = reviewed();
    let next = step(Some(&r), &detail("h2"), &[], at(11, 9));
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round_reasons, [RoundReason::NewCommits]);
    assert_eq!(next.round, 2);

    // Staying in re-review over later polls does not bump the round again.
    let later = step(Some(&next), &detail("h3"), &[], at(11, 12));
    assert_eq!(later.round, 2);

    // Marked reviewed, then more commits: round 3.
    let done = mark_reviewed(&later, &[], at(11, 13));
    assert_eq!(done.status, ReviewStatus::WaitingOnAuthor);
    assert_eq!(done.last_reviewed_sha.as_deref(), Some("h3"));
    let round3 = step(Some(&done), &detail("h4"), &[], at(12, 9));
    assert_eq!(round3.round, 3);
}

#[test]
fn a_reply_in_my_thread_starts_a_new_round() {
    let (r, d) = reviewed();
    let mine = thread(
        "1",
        ME,
        "h1",
        vec![note("1", ME, at(10, 11)), note("2", AUTHOR, at(11, 8))],
    );
    let next = step(Some(&r), &d, &[mine], at(11, 9));
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round_reasons, [RoundReason::AuthorReplied]);
}

#[test]
fn replies_that_do_not_count() {
    let (r, d) = reviewed();
    // Before the watermark.
    let old = thread(
        "1",
        ME,
        "h1",
        vec![note("1", ME, at(10, 8)), note("2", AUTHOR, at(10, 9))],
    );
    // In someone else's thread.
    let theirs = thread(
        "2",
        OTHER,
        "h1",
        vec![note("3", OTHER, at(11, 7)), note("4", AUTHOR, at(11, 8))],
    );
    // The user's own reply, and a system note.
    let mut system = note("6", AUTHOR, at(11, 8));
    system.system = true;
    let own = thread("3", ME, "h1", vec![note("5", ME, at(11, 7)), system]);
    let next = step(Some(&r), &d, &[old, theirs, own], at(11, 9));
    assert_eq!(next.status, ReviewStatus::WaitingOnAuthor);
}

#[test]
fn a_re_request_is_a_transition() {
    let (r, mut d) = reviewed();
    // The user reviewed on the platform: no longer requested.
    d.requested_from_me = false;
    let dropped = step(Some(&r), &d, &[], at(11, 8));
    assert_eq!(dropped.status, ReviewStatus::WaitingOnAuthor);
    assert!(!dropped.requested);

    d.requested_from_me = true;
    let again = step(Some(&dropped), &d, &[], at(11, 9));
    assert_eq!(again.status, ReviewStatus::NeedsReReview);
    assert_eq!(again.round_reasons, [RoundReason::ReRequested]);
    assert_eq!(again.rerequested_at, Some(at(11, 9)));

    // The reason sticks until the user marks the round reviewed.
    let later = step(Some(&again), &d, &[], at(11, 12));
    assert_eq!(later.round_reasons, [RoundReason::ReRequested]);
    let done = mark_reviewed(&later, &[], at(11, 13));
    assert_eq!(done.status, ReviewStatus::WaitingOnAuthor);
}

#[test]
fn several_reasons_at_once() {
    let (r, mut d) = reviewed();
    d.requested_from_me = false;
    let dropped = step(Some(&r), &d, &[], at(11, 8));
    let mut d = detail("h2");
    d.requested_from_me = true;
    let mine = thread(
        "1",
        ME,
        "h1",
        vec![note("1", ME, at(10, 11)), note("2", AUTHOR, at(11, 8))],
    );
    let next = step(Some(&dropped), &d, &[mine], at(11, 9));
    assert_eq!(
        next.round_reasons,
        [
            RoundReason::NewCommits,
            RoundReason::AuthorReplied,
            RoundReason::ReRequested
        ]
    );
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round, 2);
}

#[test]
fn approval_removes_the_item() {
    let (r, mut d) = reviewed();
    d.approved_by_me = true;
    d.head_sha = "h2".into(); // even with new commits
    let approved = step(Some(&r), &d, &[], at(11, 9));
    assert_eq!(approved.status, ReviewStatus::Approved);
    assert_eq!(approved.approved_at, Some(at(11, 9)));
    assert!(approved.round_reasons.is_empty());
    assert!(!approved.is_active());

    // approved_at is set once, not moved by later polls.
    let later = step(Some(&approved), &d, &[], at(12, 9));
    assert_eq!(later.approved_at, Some(at(11, 9)));

    // Merged after approval stays approved.
    d.state = ReviewState::Merged;
    assert_eq!(
        step(Some(&later), &d, &[], at(12, 10)).status,
        ReviewStatus::Approved
    );

    // The approval went away on the platform (dismissed / revoked).
    let mut d = detail("h2");
    d.approved_by_me = false;
    let revoked = step(Some(&later), &d, &[], at(12, 11));
    assert_eq!(revoked.approved_at, None);
    assert_eq!(revoked.status, ReviewStatus::NeedsReReview);

    // Approved on first sight.
    let mut d = detail("h1");
    d.approved_by_me = true;
    assert_eq!(
        step(None, &d, &[], at(10, 10)).status,
        ReviewStatus::Approved
    );
}

#[test]
fn closed_or_merged_without_approval() {
    for state in [ReviewState::Closed, ReviewState::Merged] {
        let (r, mut d) = reviewed();
        d.state = state;
        d.head_sha = "h2".into();
        let closed = step(Some(&r), &d, &[], at(11, 9));
        assert_eq!(closed.status, ReviewStatus::ClosedWithoutApproval);
        assert!(closed.round_reasons.is_empty());
        assert!(closed.is_active());
    }
    // Also when never reviewed.
    let mut d = detail("h1");
    d.state = ReviewState::Closed;
    assert_eq!(
        step(None, &d, &[], at(10, 10)).status,
        ReviewStatus::ClosedWithoutApproval
    );
}

#[test]
fn dropped_from_the_list_is_not_active() {
    let r = advance(None, &detail("h1"), &[], ME, false, at(10, 10));
    assert_eq!(r.status, ReviewStatus::NeedsReview);
    assert!(!r.is_active());
}

#[test]
fn untouched_threads() {
    let (r, d) = reviewed();
    let quiet = thread("1", ME, "h1", vec![note("1", ME, at(10, 11))]);
    let answered = thread(
        "2",
        ME,
        "h1",
        vec![note("2", ME, at(10, 11)), note("3", AUTHOR, at(11, 8))],
    );
    // The author replied, then the user answered: back on the author.
    let answered_back = thread(
        "3",
        ME,
        "h1",
        vec![
            note("4", ME, at(10, 11)),
            note("5", AUTHOR, at(11, 8)),
            note("6", ME, at(11, 9)),
        ],
    );
    let mut resolved = thread("4", ME, "h1", vec![note("7", ME, at(10, 11))]);
    resolved.resolved = true;
    let theirs = thread("5", OTHER, "h1", vec![note("8", OTHER, at(10, 11))]);
    let all = [quiet, answered, answered_back, resolved, theirs];

    let rec = step(Some(&r), &d, &all, at(11, 10));
    assert_eq!(rec.untouched_threads, 2);

    // Any new commit counts as touching every thread (M2 approximation).
    let rec = step(Some(&r), &detail("h2"), &all, at(11, 10));
    assert_eq!(rec.untouched_threads, 0);

    // Before any watermark: unchanged since the commit the thread was left on.
    let fresh = step(None, &detail("h1"), &all[..1], at(10, 12));
    assert_eq!(fresh.untouched_threads, 1);
    let moved = step(None, &detail("h2"), &all[..1], at(10, 12));
    assert_eq!(moved.untouched_threads, 0);
}

#[test]
fn reasons_and_statuses_round_trip_their_keys() {
    for r in [
        RoundReason::NewCommits,
        RoundReason::AuthorReplied,
        RoundReason::ReRequested,
    ] {
        assert_eq!(RoundReason::parse(r.as_str()), Some(r));
        assert_eq!(serde_json::to_value(r).unwrap(), r.as_str());
    }
    for s in [
        ReviewStatus::NeedsReview,
        ReviewStatus::NeedsReReview,
        ReviewStatus::WaitingOnAuthor,
        ReviewStatus::ClosedWithoutApproval,
        ReviewStatus::Approved,
    ] {
        assert_eq!(ReviewStatus::parse(s.as_str()), Some(s));
        assert_eq!(serde_json::to_value(s).unwrap(), s.as_str());
    }
}

#[test]
fn untouched_threads_with_line_walks() {
    use crate::history::{LineTouch, LineTouches};

    let (r, _) = reviewed();
    let quiet = thread("1", ME, "h1", vec![note("1", ME, at(10, 11))]);
    let rewritten = thread("2", ME, "h1", vec![note("2", ME, at(10, 11))]);
    let unwalked = thread("3", ME, "h1", vec![note("3", ME, at(10, 11))]);
    let all = [quiet, rewritten, unwalked];
    let touch = |touched_by: &[&str], usable| LineTouch {
        then_sha: "h1".into(),
        commits: ["h1", "h2", "h3"].map(String::from).to_vec(),
        touched_by: touched_by.iter().map(|s| s.to_string()).collect(),
        usable,
    };
    let touches: LineTouches = [
        ("1".to_string(), touch(&[], true)),
        ("2".to_string(), touch(&["h3"], true)),
    ]
    .into();
    let input = |head: &'static str, last: Option<&'static str>| RoundInput {
        state: ReviewState::Open,
        head_sha: head,
        last_reviewed_sha: last,
        last_reviewed_at: r.last_reviewed_at,
        approved_at: None,
        rerequested_at: None,
        threads: &all,
        me: ME,
        seen_at: None,
        my_state: MyReviewState::NotReviewer,
        platform_reviewed_sha: None,
        platform_reviewed_at: None,
    };

    // Watermark h1, head h3: the walk says thread 1's lines were left
    // alone; thread 2's were rewritten in h3; thread 3 has no walk and
    // falls back to "any new commit touches it".
    assert_eq!(untouched_with(&input("h3", Some("h1")), Some(&touches)), 1);
    // Without walks: the M2 approximation, nothing is untouched.
    assert_eq!(untouched(&input("h3", Some("h1"))), 0);
    // Watermark at h3: h3's rewrite is no longer "since".
    assert_eq!(untouched_with(&input("h3", Some("h3")), Some(&touches)), 3);
    // A walk that cannot answer falls back as well.
    let unusable: LineTouches = [("1".to_string(), touch(&[], false))].into();
    assert_eq!(untouched_with(&input("h3", Some("h1")), Some(&unusable)), 0);
    // A note from someone else still counts, walk or not.
    let mut answered = all.clone();
    answered[0].notes.push(note("9", AUTHOR, at(11, 9)));
    let input = RoundInput {
        threads: &answered,
        ..input("h3", Some("h1"))
    };
    assert_eq!(untouched_with(&input, Some(&touches)), 0);
}

#[test]
fn a_reply_between_the_last_fetch_and_the_click_still_counts() {
    // Fetched at 10:10 with the user's own thread; the author replies at
    // 10:30 (not fetched yet); the user marks it reviewed at 11:00.
    let d = detail("h1");
    let mine = thread("1", ME, "h1", vec![note("1", ME, at(10, 9))]);
    let fetched = step(None, &d, std::slice::from_ref(&mine), at(10, 10));
    let done = mark_reviewed(&fetched, std::slice::from_ref(&mine), at(11, 0));
    assert_eq!(done.last_reviewed_at, Some(at(10, 10)));
    assert_eq!(done.status, ReviewStatus::WaitingOnAuthor);

    let mut replied = mine.clone();
    replied.notes.push(note(
        "2",
        AUTHOR,
        at(10, 10) + chrono::Duration::minutes(20),
    ));
    let next = step(Some(&done), &d, &[replied], at(11, 3));
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round_reasons, [RoundReason::AuthorReplied]);
}

#[test]
fn a_reply_a_fetch_missed_is_never_taken_as_seen() {
    // The poll started at 10:10 and read the thread pages one by one: the
    // author replied at 10:10:30 on a page already read (missed), and a
    // note from 10:11 on a later page made it in. The watermark is the
    // fetch's start, so the missed reply still counts once fetched.
    let d = detail("h1");
    let mine = thread("1", ME, "h1", vec![note("1", ME, at(10, 9))]);
    let other = thread(
        "2",
        AUTHOR,
        "h1",
        vec![note("2", AUTHOR, at(10, 10) + chrono::Duration::minutes(1))],
    );
    let stored = [mine.clone(), other.clone()];
    let fetched = step(None, &d, &stored, at(10, 10));
    let done = mark_reviewed(&fetched, &stored, at(12, 0));
    assert_eq!(done.last_reviewed_at, Some(at(10, 10)));

    let mut replied = mine;
    replied.notes.push(note(
        "3",
        AUTHOR,
        at(10, 10) + chrono::Duration::seconds(30),
    ));
    let next = step(Some(&done), &d, &[replied, other], at(12, 5));
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round_reasons, [RoundReason::AuthorReplied]);
}

#[test]
fn the_watermark_is_the_last_fetch_and_never_after_the_click() {
    let d = detail("h1");
    let fetched = step(None, &d, &[], at(10, 10));
    assert_eq!(
        mark_reviewed(&fetched, &[], at(12, 0)).last_reviewed_at,
        Some(at(10, 10))
    );
    let click = at(10, 10) - chrono::Duration::seconds(30);
    assert_eq!(
        mark_reviewed(&fetched, &[], click).last_reviewed_at,
        Some(click)
    );
}

#[test]
fn a_thread_i_replied_in_is_mine_for_the_round() {
    let (r, d) = reviewed();
    // A colleague's thread: their note and the author's after the
    // watermark, then the user's reply. Only what comes after the user's
    // own last note is new to them.
    let mut replied = thread(
        "1",
        OTHER,
        "h1",
        vec![
            note("1", OTHER, at(10, 9)),
            note("2", AUTHOR, at(11, 7)),
            note("3", ME, at(11, 8)),
        ],
    );
    let quiet = step(Some(&r), &d, std::slice::from_ref(&replied), at(11, 9));
    assert_eq!(quiet.status, ReviewStatus::WaitingOnAuthor);
    // Untouched since the user's reply: it counts like an own thread.
    assert_eq!(quiet.untouched_threads, 1);

    replied.notes.push(note("4", AUTHOR, at(11, 10)));
    let next = step(Some(&quiet), &d, &[replied], at(11, 11));
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round_reasons, [RoundReason::AuthorReplied]);
    assert_eq!(next.untouched_threads, 0);
}

#[test]
fn a_system_note_of_mine_does_not_make_a_thread_mine() {
    let (r, d) = reviewed();
    // The user resolved a colleague's thread; the author answered after.
    let mut resolve = note("2", ME, at(11, 7));
    resolve.system = true;
    let t = thread(
        "1",
        OTHER,
        "h1",
        vec![
            note("1", OTHER, at(10, 9)),
            resolve,
            note("3", AUTHOR, at(11, 8)),
        ],
    );
    let next = step(Some(&r), &d, &[t], at(11, 9));
    assert_eq!(next.status, ReviewStatus::WaitingOnAuthor);
    assert_eq!(next.untouched_threads, 0);
}

#[test]
fn a_reply_the_user_already_answered_is_not_new() {
    let (r, d) = reviewed();
    // In the user's own thread: the author replied after the watermark and
    // the user answered that before the fetch.
    let mine = thread(
        "1",
        ME,
        "h1",
        vec![
            note("1", ME, at(10, 11)),
            note("2", AUTHOR, at(11, 7)),
            note("3", ME, at(11, 8)),
        ],
    );
    let next = step(Some(&r), &d, &[mine], at(11, 9));
    assert_eq!(next.status, ReviewStatus::WaitingOnAuthor);
}

#[test]
fn a_system_note_of_mine_does_not_answer_a_reply() {
    let (r, d) = reviewed();
    // The user resolved their thread on the platform after the author's
    // reply: that is no answer to it.
    let mut resolve = note("3", ME, at(11, 8));
    resolve.system = true;
    let mine = thread(
        "1",
        ME,
        "h1",
        vec![
            note("1", ME, at(10, 11)),
            note("2", AUTHOR, at(11, 7)),
            resolve,
        ],
    );
    let next = step(Some(&r), &d, &[mine], at(11, 9));
    assert_eq!(next.round_reasons, [RoundReason::AuthorReplied]);
}

#[test]
fn a_note_written_over_an_older_view_answers_only_what_that_view_showed() {
    let (r, _) = reviewed();
    // The user's view was fetched at 11:00; the author replied at 11:07,
    // and the user's note at 11:08 was written over the 11:00 view.
    let threads = [thread(
        "1",
        ME,
        "h1",
        vec![
            note("1", ME, at(10, 11)),
            note("2", AUTHOR, at(11, 7)),
            note("3", ME, at(11, 8)),
        ],
    )];
    let input = |seen_at| RoundInput {
        state: ReviewState::Open,
        head_sha: "h1",
        last_reviewed_sha: Some("h1"),
        last_reviewed_at: r.last_reviewed_at,
        approved_at: None,
        rerequested_at: None,
        threads: &threads,
        me: ME,
        seen_at,
        my_state: MyReviewState::NotReviewer,
        platform_reviewed_sha: None,
        platform_reviewed_at: None,
    };
    let unseen = classify(&input(Some(at(11, 0))));
    assert_eq!(unseen.reasons, [RoundReason::AuthorReplied]);
    assert_eq!(unseen.untouched_threads, 0);
    // A view from after the reply showed it: answered.
    let seen = classify(&input(Some(at(11, 7) + chrono::Duration::seconds(30))));
    assert_eq!(seen.status, ReviewStatus::WaitingOnAuthor);
    // Unknown (a note posted on the platform): it answers what came before.
    assert_eq!(classify(&input(None)).status, ReviewStatus::WaitingOnAuthor);
}

#[test]
fn a_reply_found_unanswered_stays_so_until_the_user_acts() {
    let (r, d) = reviewed();
    // Found unanswered (after a post over an older view, say) by the
    // fetch at 11:09, which also stored the user's note.
    let threads = vec![thread(
        "1",
        ME,
        "h1",
        vec![
            note("1", ME, at(10, 11)),
            note("2", AUTHOR, at(11, 7)),
            note("3", ME, at(11, 8)),
        ],
    )];
    let mut flagged = step(Some(&r), &d, &threads, at(11, 9));
    flagged.status = ReviewStatus::NeedsReReview;
    flagged.round_reasons = vec![RoundReason::AuthorReplied];
    flagged.round = 2;
    flagged.untouched_threads = 0;
    // Later fetches would take the 11:08 note as the answer: nothing the
    // user did since says so.
    let later = step_over(&flagged, &threads, &d, &threads, at(11, 12));
    assert_eq!(later.status, ReviewStatus::NeedsReReview);
    assert_eq!(later.round_reasons, [RoundReason::AuthorReplied]);
    assert_eq!(later.round, 2);
    // And the thread with the reply is not "untouched" either.
    assert_eq!(later.untouched_threads, 0);
    // A new note of the user's does.
    let mut answered = threads.clone();
    answered[0].notes.push(note("4", ME, at(11, 13)));
    let after = step_over(&later, &threads, &d, &answered, at(11, 15));
    assert_eq!(after.status, ReviewStatus::WaitingOnAuthor);
    assert_eq!(after.untouched_threads, 1);
    // And so does marking it reviewed.
    let done = mark_reviewed(&later, &threads, at(11, 14));
    assert_eq!(
        step_over(&done, &threads, &d, &threads, at(11, 15)).status,
        ReviewStatus::WaitingOnAuthor
    );
}

#[test]
fn what_the_user_wrote_since_is_told_by_note_ids_not_clocks() {
    let (r, d) = reviewed();
    // The platform's clock runs ahead of this machine's: the user's note
    // stored by the 11:09 fetch carries 11:10.
    let threads = vec![thread(
        "1",
        ME,
        "h1",
        vec![
            note("1", ME, at(10, 11)),
            note("2", AUTHOR, at(11, 7)),
            note("3", ME, at(11, 10)),
        ],
    )];
    let mut flagged = step(Some(&r), &d, &threads, at(11, 9));
    flagged.status = ReviewStatus::NeedsReReview;
    flagged.round_reasons = vec![RoundReason::AuthorReplied];
    let later = step_over(&flagged, &threads, &d, &threads, at(11, 12));
    assert_eq!(later.round_reasons, [RoundReason::AuthorReplied]);
    // A note the stored threads lack is new, whatever its time (a clock
    // behind, or a note the last fetch did not bring).
    let mut answered = threads.clone();
    answered[0].notes.push(note("4", ME, at(11, 5)));
    let after = step_over(&later, &threads, &d, &answered, at(11, 15));
    assert_eq!(after.status, ReviewStatus::WaitingOnAuthor);
}

#[test]
fn the_untouched_count_agrees_with_a_reply_kept_unanswered() {
    let threads = [thread(
        "1",
        ME,
        "h1",
        vec![
            note("1", ME, at(10, 11)),
            note("2", AUTHOR, at(11, 7)),
            note("3", ME, at(11, 8)),
        ],
    )];
    let input = RoundInput {
        state: ReviewState::Open,
        head_sha: "h1",
        last_reviewed_sha: Some("h1"),
        last_reviewed_at: Some(at(11, 0)),
        approved_at: None,
        rerequested_at: None,
        threads: &threads,
        me: ME,
        seen_at: None,
        my_state: MyReviewState::NotReviewer,
        platform_reviewed_sha: None,
        platform_reviewed_at: None,
    };
    // Read as the notes say, the 11:08 note answered the reply.
    assert_eq!(untouched_agreeing(&input, &[], None), 1);
    // Kept unanswered: written over the view at the watermark.
    assert_eq!(
        untouched_agreeing(&input, &[RoundReason::AuthorReplied], None),
        0
    );
    // A reply the notes show unanswered needs no second reading.
    let mut open = threads.clone();
    open[0].notes.pop();
    let input = RoundInput {
        threads: &open,
        ..input
    };
    assert_eq!(
        untouched_agreeing(&input, &[RoundReason::AuthorReplied], None),
        0
    );
}

// ------------------------------------------- the platform's reviewer state

/// A fetch where the platform has the user's review in `state`.
fn with_state(head: &str, state: MyReviewState) -> ReviewDetail {
    let mut d = detail(head);
    d.my_review_state = state;
    d.requested_from_me = state.owes_review();
    d
}

#[test]
fn awaiting_review_with_no_review_given_needs_review() {
    let d = with_state("h1", MyReviewState::AwaitingReview);
    let r = step(None, &d, &[], at(10, 10));
    assert_eq!(r.status, ReviewStatus::NeedsReview);
    assert!(r.round_reasons.is_empty());
    assert_eq!(r.my_review_state, MyReviewState::AwaitingReview);
    // Posting (or an old local mark) moves Argus' watermark, but the
    // platform still owes the first review: it stays "Needs review".
    let marked = mark_reviewed(&r, &[], at(10, 12));
    assert_eq!(marked.status, ReviewStatus::NeedsReview);
    let again = step(Some(&marked), &d, &[], at(10, 13));
    assert_eq!(again.status, ReviewStatus::NeedsReview);
    assert!(again.is_active());
}

/// !15210: marked reviewed locally at the head, requested at both fetches
/// (so no false→true edge), and GitLab's "requested review from @me" note
/// dated after the mark. It needs the user again, whatever the watermark.
#[test]
fn a_re_request_after_the_watermark_needs_re_review_without_an_edge() {
    let mut d = with_state("h1", MyReviewState::AwaitingReview);
    // The first request, before anything of the user's.
    d.review_requested_at = Some(at(10, 9));
    let first = step(None, &d, &[], at(10, 10));
    assert_eq!(first.status, ReviewStatus::NeedsReview);
    let marked = mark_reviewed(&first, &[], at(10, 11));
    assert_eq!(marked.last_reviewed_sha.as_deref(), Some("h1"));
    assert_eq!(marked.status, ReviewStatus::NeedsReview);

    // Re-requested at 11:00 the next day: still requested, same head.
    d.review_requested_at = Some(at(11, 11));
    let next = step(Some(&marked), &d, &[], at(11, 12));
    assert!(marked.requested && next.requested, "no transition to catch");
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round_reasons, [RoundReason::ReRequested]);
    assert_eq!(next.rerequested_at, Some(at(11, 11)));
    assert_eq!(next.round, 2);
    // It stays so on later polls, and the round does not count up again.
    let later = step(Some(&next), &d, &[], at(11, 15));
    assert_eq!(later.status, ReviewStatus::NeedsReReview);
    assert_eq!(later.round, 2);
}

#[test]
fn owed_again_after_a_review_on_the_platform_needs_re_review() {
    // GitHub: the user commented on h1 on the 10th, the author pushed h2
    // and asked again. No local watermark at all.
    let mut d = with_state("h2", MyReviewState::AwaitingReview);
    d.my_reviewed_at = Some(at(10, 12));
    d.my_reviewed_sha = Some("h1".into());
    let r = step(None, &d, &[], at(11, 9));
    assert_eq!(r.status, ReviewStatus::NeedsReReview);
    assert_eq!(
        r.round_reasons,
        [RoundReason::NewCommits, RoundReason::ReRequested]
    );
    // The same head as reviewed: re-requested only.
    let mut same = d.clone();
    same.head_sha = "h1".into();
    let r = step(None, &same, &[], at(11, 9));
    assert_eq!(r.round_reasons, [RoundReason::ReRequested]);
}

#[test]
fn changes_requested_on_the_platform_wait_on_the_author_until_new_commits() {
    // Requested on GitLab itself at h1, 10-10 12:00; Argus never marked it.
    let mut d = with_state("h1", MyReviewState::RequestedChanges);
    d.my_reviewed_at = Some(at(10, 12));
    d.my_reviewed_sha = Some("h1".into());
    let r = step(None, &d, &[], at(10, 13));
    assert_eq!(r.status, ReviewStatus::WaitingOnAuthor);
    assert!(r.is_active());
    assert_eq!(r.last_reviewed_sha, None, "Argus' own watermark untouched");

    // The author pushes h2: re-review, for the new commits.
    let mut pushed = d.clone();
    pushed.head_sha = "h2".into();
    let next = step(Some(&r), &pushed, &[], at(11, 9));
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round_reasons, [RoundReason::NewCommits]);

    // Or the author answers in the user's thread after that review.
    let mine = thread(
        "1",
        ME,
        "h1",
        vec![note("1", ME, at(10, 11)), note("2", AUTHOR, at(10, 14))],
    );
    let replied = step(Some(&r), &d, &[mine], at(10, 15));
    assert_eq!(replied.status, ReviewStatus::NeedsReReview);
    assert_eq!(replied.round_reasons, [RoundReason::AuthorReplied]);
}

#[test]
fn the_later_of_argus_and_the_platform_is_the_watermark() {
    // Marked in Argus at h1 (10-10 10:00); later commented on GitHub at h2.
    let (r, _) = reviewed();
    let mut d = with_state("h2", MyReviewState::Commented);
    d.my_reviewed_at = Some(at(11, 9));
    d.my_reviewed_sha = Some("h2".into());
    let next = step(Some(&r), &d, &[], at(11, 10));
    assert_eq!(next.status, ReviewStatus::WaitingOnAuthor);
    // An older platform review leaves Argus' newer watermark in charge.
    let mut older = with_state("h2", MyReviewState::Commented);
    older.my_reviewed_at = Some(at(9, 9));
    older.my_reviewed_sha = Some("h0".into());
    let next = step(Some(&r), &older, &[], at(11, 10));
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round_reasons, [RoundReason::NewCommits]);
}

#[test]
fn a_verdict_without_a_date_waits_on_the_author() {
    let d = with_state("h1", MyReviewState::Commented);
    let r = step(None, &d, &[], at(10, 10));
    assert_eq!(r.status, ReviewStatus::WaitingOnAuthor);
    let d = with_state("h1", MyReviewState::NotReviewer);
    assert_eq!(
        step(None, &d, &[], at(10, 10)).status,
        ReviewStatus::NeedsReview
    );
}

#[test]
fn an_approval_leaves_and_comes_back_when_reset() {
    let mut d = with_state("h1", MyReviewState::Approved);
    d.approved_by_me = true;
    d.my_reviewed_at = Some(at(10, 12));
    d.my_reviewed_sha = Some("h1".into());
    let approved = step(None, &d, &[], at(10, 13));
    assert_eq!(approved.status, ReviewStatus::Approved);
    assert!(!approved.is_active());

    // A push resets the approval and asks again: back in the lists.
    let mut reset = with_state("h2", MyReviewState::AwaitingReview);
    reset.my_reviewed_at = Some(at(10, 12));
    reset.my_reviewed_sha = Some("h1".into());
    let back = step(Some(&approved), &reset, &[], at(11, 9));
    assert_eq!(back.approved_at, None);
    assert_eq!(back.status, ReviewStatus::NeedsReReview);
    assert_eq!(
        back.round_reasons,
        [RoundReason::NewCommits, RoundReason::ReRequested]
    );
    assert!(back.is_active());
}

#[test]
fn the_request_time_never_moves_back() {
    let mut d = with_state("h1", MyReviewState::AwaitingReview);
    d.review_requested_at = Some(at(11, 9));
    let r = step(None, &d, &[], at(11, 10));
    assert_eq!(r.rerequested_at, Some(at(11, 9)));
    // A later read that dates nothing (the notes failed) keeps it.
    d.review_requested_at = None;
    let next = step(Some(&r), &d, &[], at(11, 11));
    assert_eq!(next.rerequested_at, Some(at(11, 9)));
}
