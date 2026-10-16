//! One ordered timeline per thread: its notes, the commits that touched
//! its lines, and resolve events.

use super::walk::LineEvent;
use super::{ThreadResolution, TimelineEntry, TimelineEvent};
use crate::provider::{CommitInfo, Thread};

/// Every note of the thread (system notes included, flagged), the first as
/// the root.
pub fn note_entries(thread: &Thread) -> Vec<TimelineEntry> {
    thread
        .notes
        .iter()
        .enumerate()
        .map(|(i, n)| TimelineEntry {
            at: Some(n.created_at),
            event: TimelineEvent::Note {
                note_id: n.id.clone(),
                author: n.author.clone(),
                body: n.body.clone(),
                system: n.system,
                root: i == 0,
            },
        })
        .collect()
}

/// The walk's events that name a commit, with that commit's date, title
/// and author from the review's commit list (blank when it is not there).
pub fn commit_entries(events: &[LineEvent], commits: &[CommitInfo]) -> Vec<TimelineEntry> {
    events
        .iter()
        .filter_map(|e| {
            let sha = e.sha.as_deref()?;
            let info = commits.iter().find(|c| c.sha == sha);
            Some(TimelineEntry {
                at: info.and_then(|c| c.authored_at),
                event: TimelineEvent::Commit {
                    sha: sha.to_string(),
                    title: info.map(|c| c.title.clone()).unwrap_or_default(),
                    author: info.map(|c| c.author.clone()).unwrap_or_default(),
                    change: e.change.clone(),
                },
            })
        })
        .collect()
}

pub fn resolution_entries(resolutions: &[ThreadResolution]) -> Vec<TimelineEntry> {
    resolutions
        .iter()
        .map(|r| TimelineEntry {
            at: Some(r.at),
            event: if r.resolved {
                TimelineEvent::Resolved { by: r.by.clone() }
            } else {
                TimelineEvent::Unresolved { by: r.by.clone() }
            },
        })
        .collect()
}

/// Sort by time, oldest first; entries without a time go last. The sort is
/// stable, so ties keep the order they came in (notes, then commits, then
/// resolve events, each in their own order).
pub fn merge(mut entries: Vec<TimelineEntry>) -> Vec<TimelineEntry> {
    entries.sort_by_key(|e| (e.at.is_none(), e.at));
    entries
}
