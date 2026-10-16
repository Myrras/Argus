//! Builders for unit tests: fictitious reviews, threads and notes.

use chrono::{DateTime, TimeZone, Utc};

use crate::provider::{
    Anchor, Note, ProviderId, ReviewDetail, ReviewRef, ReviewState, Side, Thread,
};

pub const ME: &str = "rev-fixture";
pub const AUTHOR: &str = "alice-fixture";

/// 2026-10-<day> <hour>:00 UTC.
pub fn at(day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap()
}

pub fn review_ref(number: u64) -> ReviewRef {
    ReviewRef {
        provider: ProviderId::Github,
        host: "github.com".into(),
        repo: "acme-fixture/widgets".into(),
        number,
        url: format!("https://github.com/acme-fixture/widgets/pull/{number}"),
        title: "Add widget caching".into(),
        author: AUTHOR.into(),
        updated_at: Some(at(10, 9)),
    }
}

pub fn detail(head: &str) -> ReviewDetail {
    ReviewDetail {
        review: review_ref(12),
        description: String::new(),
        state: ReviewState::Open,
        draft: false,
        head_sha: head.into(),
        base_sha: "base".into(),
        start_sha: None,
        commits: Vec::new(),
        versions: Vec::new(),
        reviewers: Vec::new(),
        requested_from_me: true,
        approved_by_me: false,
    }
}

pub fn note(id: &str, author: &str, when: DateTime<Utc>) -> Note {
    Note {
        id: id.into(),
        author: author.into(),
        body: format!("note {id}"),
        created_at: when,
        system: false,
    }
}

/// A thread started by `author` on commit `sha` with `notes` (the first is
/// the root).
pub fn thread(id: &str, author: &str, sha: &str, notes: Vec<Note>) -> Thread {
    Thread {
        id: id.into(),
        author: author.into(),
        created_at: notes.first().map_or(at(1, 0), |n| n.created_at),
        resolved: false,
        outdated: false,
        anchor: Some(Anchor {
            path_old: Some("src/cache.rs".into()),
            path_new: Some("src/cache.rs".into()),
            line_old: None,
            line_new: Some(10),
            start_line: None,
            side: Side::New,
            base_sha: None,
            start_sha: None,
            head_sha: sha.into(),
            diff_hunk: None,
            current_line: None,
        }),
        notes,
    }
}
