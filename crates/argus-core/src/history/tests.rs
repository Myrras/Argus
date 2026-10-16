use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::cache::{CachedHistory, CachedMapping};
use super::patch::{map_line, parse_blocks, touches, Block, Fate};
use super::snapshot::{around, diff_rows, from_diff_hunk, window};
use super::timeline::merge;
use super::walk::LineEvent;
use super::*;
use crate::error::{CoreError, CoreResult};
use crate::provider::{
    ChangedFile, CommitInfo, DiffRange, FileDiff, FileStatus, ProviderId, ReviewDetail,
    ReviewProvider, ReviewRef, Side, Thread,
};
use crate::review::ReviewStatus;
use crate::store::Store;
use crate::testutil::{at, detail, note, thread, AUTHOR, ME};
use crate::Core;

// ---------------------------------------------------------------- patches

#[test]
fn parses_blocks_with_headers_markers_and_zero_lengths() {
    let patch = "diff --git a/x b/x\nindex 1..2 100644\n--- a/x\n+++ b/x\n\
                 @@ -1,4 +1,5 @@\n a\n-b\n+B\n+B2\n c\n d\n\
                 @@ -10,0 +12,2 @@ fn f() {\n+n1\n+n2\n\
                 @@ -20,2 +21,0 @@\n-x\n-y\n\\ No newline at end of file\n";
    let blocks = parse_blocks(patch).unwrap();
    assert_eq!(
        blocks,
        [
            Block {
                old: 2,
                removed: 1,
                new: 2,
                added: 2
            },
            // -10,0: inserted after old line 10, i.e. before old line 11.
            Block {
                old: 11,
                removed: 0,
                new: 12,
                added: 2
            },
            // +21,0: deleted after new line 21; the next line is new 22.
            Block {
                old: 20,
                removed: 2,
                new: 22,
                added: 0
            },
        ]
    );
    // A hunk that promises more lines than it has is not a patch.
    assert!(parse_blocks("@@ -1,3 +1,3 @@\n a\n").is_none());
    assert!(parse_blocks("@@ nonsense @@\n a\n").is_none());
    // A missing length is 1; a context line whose space was trimmed counts.
    let b = parse_blocks("@@ -5 +5 @@\n-old\n+new").unwrap();
    assert_eq!(map_line(&b, 5), Fate::Changed(5));
    let b = parse_blocks("@@ -1,3 +1,3 @@\n a\n\n-c\n+C").unwrap();
    assert_eq!(map_line(&b, 3), Fate::Changed(3));
}

/// A patch over a 30-line file: 2 lines inserted before line 5, line 10
/// replaced by two, lines 20-21 removed.
fn sample_blocks() -> Vec<Block> {
    parse_blocks(
        "@@ -3,4 +3,6 @@\n c3\n c4\n+i1\n+i2\n c5\n c6\n\
         @@ -8,5 +10,6 @@\n c8\n c9\n-old10\n+new10a\n+new10b\n c11\n c12\n\
         @@ -17,7 +20,5 @@\n c17\n c18\n c19\n-d20\n-d21\n c22\n c23\n",
    )
    .unwrap()
}

#[test]
fn maps_lines_above_below_and_inside_hunks() {
    let b = sample_blocks();
    // Above every change.
    assert_eq!(map_line(&b, 1), Fate::Kept(1));
    assert_eq!(map_line(&b, 4), Fate::Kept(4));
    // Right at the insertion point: pushed down, not touched.
    assert_eq!(map_line(&b, 5), Fate::Kept(7));
    assert!(!touches(&b, 5, 5));
    assert!(!touches(&b, 4, 4));
    // Between hunks: shifted by the insertion.
    assert_eq!(map_line(&b, 9), Fate::Kept(11));
    // Inside: replaced by the first of its two lines.
    assert_eq!(map_line(&b, 10), Fate::Changed(12));
    assert!(touches(&b, 10, 10));
    // Edges of the replaced line.
    assert_eq!(map_line(&b, 11), Fate::Kept(14));
    assert!(!touches(&b, 9, 9) && !touches(&b, 11, 11));
    // Removed lines: nothing in their place, near the line above.
    assert_eq!(map_line(&b, 20), Fate::Removed { near: 22 });
    assert_eq!(map_line(&b, 21), Fate::Removed { near: 22 });
    // Below the last hunk.
    assert_eq!(map_line(&b, 22), Fate::Kept(23));
    assert_eq!(map_line(&b, 30), Fate::Kept(31));
}

#[test]
fn ranges_are_touched_by_changes_inside_not_beside() {
    let b = sample_blocks();
    // 4..=6 contains the insertion point between 4 and 5.
    assert!(touches(&b, 4, 6));
    // 5..=9 starts at the insertion point: lines went in above it.
    assert!(!touches(&b, 5, 9));
    // Overlaps the removed 20-21 at one end.
    assert!(touches(&b, 21, 25));
    assert!(touches(&b, 15, 20));
    assert!(!touches(&b, 22, 30));
}

// ------------------------------------------------------------------ walks

fn changed(path: &str, patch: &str) -> ChangedFile {
    ChangedFile {
        path: path.into(),
        old_path: None,
        status: FileStatus::Modified,
        additions: 0,
        deletions: 0,
        patch: Some(patch.into()),
        content_sha: path.into(),
        binary: false,
        too_large: false,
    }
}

fn renamed(from: &str, to: &str, patch: Option<&str>) -> ChangedFile {
    ChangedFile {
        old_path: Some(from.into()),
        status: FileStatus::Renamed,
        patch: patch.map(str::to_string),
        ..changed(to, "")
    }
}

fn removed(path: &str) -> ChangedFile {
    ChangedFile {
        status: FileStatus::Removed,
        patch: None,
        ..changed(path, "")
    }
}

fn diff(parent: &str, sha: &str, files: Vec<ChangedFile>) -> FileDiff {
    FileDiff {
        base_sha: parent.into(),
        head_sha: sha.into(),
        files,
        truncated: false,
    }
}

/// A scripted history: commits in order (the first is where comments are
/// made), each with its diff, and optional direct diffs.
#[derive(Default)]
struct MemSource {
    commits: Vec<String>,
    diffs: HashMap<String, FileDiff>,
    direct: HashMap<(String, String), FileDiff>,
    fetched: Mutex<Vec<String>>,
}

impl MemSource {
    fn new(steps: Vec<(&str, Vec<ChangedFile>)>) -> Self {
        let mut s = MemSource {
            commits: vec!["c0".into()],
            ..Default::default()
        };
        for (sha, files) in steps {
            let parent = s.commits.last().unwrap().clone();
            s.diffs.insert(sha.into(), diff(&parent, sha, files));
            s.commits.push(sha.into());
        }
        s
    }
}

impl LineHistorySource for MemSource {
    fn commits_between(&self, from: &str, to: &str) -> CoreResult<Option<Vec<String>>> {
        let pos = |s: &str| self.commits.iter().position(|c| c == s);
        Ok(match (pos(from), pos(to)) {
            (Some(f), Some(t)) if f < t => Some(self.commits[f + 1..=t].to_vec()),
            _ => None,
        })
    }
    fn commit_diff(&self, sha: &str) -> CoreResult<FileDiff> {
        self.fetched.lock().unwrap().push(sha.into());
        self.diffs.get(sha).cloned().ok_or(CoreError::NotFound)
    }
    fn direct_diff(&self, from: &str, to: &str) -> CoreResult<Option<FileDiff>> {
        Ok(self.direct.get(&(from.into(), to.into())).cloned())
    }
}

fn start(path: &str, start_line: u32, line: u32) -> WalkStart {
    WalkStart {
        sha: "c0".into(),
        path: path.into(),
        start_line,
        line,
    }
}

fn located(path: &str, start_line: u32, line: u32) -> (String, u32, u32) {
    (path.into(), start_line, line)
}

fn where_(m: &LineMapping) -> (String, u32, u32) {
    match &m.status {
        LineStatus::Located {
            path,
            start_line,
            line,
            ..
        } => (path.clone(), *start_line, *line),
        other => panic!("not located: {other:?}"),
    }
}

#[test]
fn walks_across_several_commits_and_attributes_touches() {
    let src = MemSource::new(vec![
        // Two lines in above line 10: shifted.
        ("c1", vec![changed("a.rs", "@@ -2,0 +3,2 @@\n+x\n+y")]),
        // Another file only.
        ("c2", vec![changed("b.rs", "@@ -1 +1 @@\n-p\n+q")]),
        // The line (now 12) rewritten.
        ("c3", vec![changed("a.rs", "@@ -12 +12 @@\n-ten\n+TEN")]),
        // A line removed above it.
        ("c4", vec![changed("a.rs", "@@ -1,2 +1 @@\n-one\n two")]),
    ]);
    let m = walk_line(&src, &start("a.rs", 10, 10), "c4").unwrap();
    assert_eq!(where_(&m), located("a.rs", 11, 11));
    assert_eq!(m.touched_by, ["c3"]);
    assert!(m.attributed);
    assert!(matches!(
        m.status,
        LineStatus::Located {
            moved: true,
            touched: true,
            renamed: false,
            ..
        }
    ));
    assert_eq!(
        m.events,
        [LineEvent {
            sha: Some("c3".into()),
            change: LineChange::Modified
        }]
    );
    // A walk to an earlier head stops there.
    let m = walk_line(&src, &start("a.rs", 10, 10), "c2").unwrap();
    assert_eq!(where_(&m), located("a.rs", 12, 12));
    assert!(m.touched_by.is_empty());
    // Already at head: nothing to fetch.
    src.fetched.lock().unwrap().clear();
    let m = walk_line(&src, &start("a.rs", 10, 10), "c0").unwrap();
    assert_eq!(where_(&m), located("a.rs", 10, 10));
    assert!(matches!(m.status, LineStatus::Located { moved: false, .. }));
    assert!(src.fetched.lock().unwrap().is_empty());
}

#[test]
fn multi_line_ranges_shrink_grow_and_survive_partial_deletes() {
    let src = MemSource::new(vec![
        // Line 6 of 5..=8 removed: the range keeps 5, 7, 8 (now 5..=7).
        (
            "c1",
            vec![changed("a.rs", "@@ -5,3 +5,2 @@\n five\n-six\n seven")],
        ),
        // A line inserted inside the range: touched, the range spans it.
        (
            "c2",
            vec![changed("a.rs", "@@ -5,2 +5,3 @@\n five\n+new\n seven")],
        ),
    ]);
    let m = walk_line(&src, &start("a.rs", 5, 8), "c1").unwrap();
    assert_eq!(where_(&m), located("a.rs", 5, 7));
    assert_eq!(m.touched_by, ["c1"]);
    let m = walk_line(&src, &start("a.rs", 5, 8), "c2").unwrap();
    assert_eq!(where_(&m), located("a.rs", 5, 8));
    assert_eq!(m.touched_by, ["c1", "c2"]);
}

#[test]
fn follows_renames_with_and_without_changes() {
    let src = MemSource::new(vec![
        ("c1", vec![renamed("old.rs", "mid.rs", None)]),
        (
            "c2",
            vec![renamed("mid.rs", "new.rs", Some("@@ -0,0 +1 @@\n+use x;"))],
        ),
        // A change to a file that now has the tracked file's old name is
        // not the tracked file.
        ("c3", vec![changed("old.rs", "@@ -1 +1 @@\n-a\n+b")]),
    ]);
    let m = walk_line(&src, &start("old.rs", 4, 4), "c3").unwrap();
    assert_eq!(where_(&m), located("new.rs", 5, 5));
    assert!(m.touched_by.is_empty());
    assert!(matches!(
        m.status,
        LineStatus::Located {
            renamed: true,
            moved: true,
            touched: false,
            ..
        }
    ));
    let renames: Vec<_> = m
        .events
        .iter()
        .map(|e| (e.sha.as_deref().unwrap(), &e.change))
        .collect();
    assert_eq!(
        renames,
        [
            (
                "c1",
                &LineChange::Renamed {
                    from: "old.rs".into(),
                    to: "mid.rs".into()
                }
            ),
            (
                "c2",
                &LineChange::Renamed {
                    from: "mid.rs".into(),
                    to: "new.rs".into()
                }
            ),
        ]
    );
}

#[test]
fn a_deleted_line_names_its_commit_and_keeps_its_place() {
    let src = MemSource::new(vec![
        (
            "c1",
            vec![changed("a.rs", "@@ -9,3 +9,2 @@\n nine\n-ten\n eleven")],
        ),
        // Two lines go in above where it was.
        ("c2", vec![changed("a.rs", "@@ -0,0 +1,2 @@\n+x\n+y")]),
    ]);
    let m = walk_line(&src, &start("a.rs", 10, 10), "c2").unwrap();
    assert_eq!(
        m.status,
        LineStatus::Deleted {
            path: "a.rs".into(),
            removed_by: Some("c1".into())
        }
    );
    assert_eq!(m.touched_by, ["c1"]);
    // It was just below line 9, which is now line 11.
    assert_eq!(m.near, Some(11));
}

#[test]
fn a_removed_file_is_gone_and_stops_the_walk() {
    let src = MemSource::new(vec![
        ("c1", vec![renamed("a.rs", "b.rs", None)]),
        ("c2", vec![removed("b.rs")]),
        ("c3", vec![changed("b.rs", "@@ -1 +1 @@\n-a\n+b")]),
    ]);
    let m = walk_line(&src, &start("a.rs", 3, 3), "c3").unwrap();
    assert_eq!(
        m.status,
        LineStatus::FileGone {
            path: "b.rs".into(),
            removed_by: Some("c2".into())
        }
    );
    assert_eq!(m.touched_by, ["c2"]);
    // c3 was never needed.
    assert!(!src.fetched.lock().unwrap().contains(&"c3".to_string()));

    // Lines deleted before the file went stay "deleted" by their commit.
    let src = MemSource::new(vec![
        ("c1", vec![changed("a.rs", "@@ -3 +2,0 @@\n-three")]),
        ("c2", vec![removed("a.rs")]),
    ]);
    let m = walk_line(&src, &start("a.rs", 3, 3), "c2").unwrap();
    assert_eq!(
        m.status,
        LineStatus::Deleted {
            path: "a.rs".into(),
            removed_by: Some("c1".into())
        }
    );
}

#[test]
fn what_cannot_be_followed_is_unlocated() {
    let unlocated = |m: LineMapping| match m.status {
        LineStatus::Unlocated { reason } => reason,
        other => panic!("expected unlocated, got {other:?}"),
    };
    let mut big = changed("a.rs", "");
    big.patch = None;
    big.too_large = true;
    let src = MemSource::new(vec![("c1", vec![big])]);
    let r = unlocated(walk_line(&src, &start("a.rs", 3, 3), "c1").unwrap());
    assert!(r.contains("too large"));

    let mut bin = changed("a.rs", "");
    bin.patch = None;
    bin.binary = true;
    let src = MemSource::new(vec![("c1", vec![bin])]);
    assert!(unlocated(walk_line(&src, &start("a.rs", 3, 3), "c1").unwrap()).contains("binary"));

    // The file is not in a cut-off file list: cannot tell.
    let mut src = MemSource::new(vec![("c1", vec![changed("b.rs", "@@ -1 +1 @@\n-a\n+b")])]);
    src.diffs.get_mut("c1").unwrap().truncated = true;
    assert!(unlocated(walk_line(&src, &start("a.rs", 3, 3), "c1").unwrap()).contains("more files"));

    let src = MemSource::new(vec![("c1", vec![changed("a.rs", "@@ -1,9 +1,9 @@\n a\n")])]);
    assert!(unlocated(walk_line(&src, &start("a.rs", 3, 3), "c1").unwrap()).contains("read"));

    // Rebased away and no direct diff: never a guessed line.
    let src = MemSource::new(vec![("c1", Vec::new())]);
    let m = walk_line(
        &src,
        &WalkStart {
            sha: "gone".into(),
            ..start("a.rs", 3, 3)
        },
        "c1",
    )
    .unwrap();
    assert!(unlocated(m).contains("rebased"));

    // A provider failure is an error, for the caller to retry.
    let src = MemSource::new(Vec::new());
    let mut src = src;
    src.commits.push("c1".into());
    assert!(walk_line(&src, &start("a.rs", 3, 3), "c1").is_err());
}

#[test]
fn rewritten_or_branched_history_uses_one_direct_diff() {
    // c2's parent is c0, not c1: a merged side branch. The direct diff
    // places the line but names no commit.
    let mut src = MemSource::new(vec![
        ("c1", vec![changed("a.rs", "@@ -0,0 +1 @@\n+x")]),
        ("c2", vec![changed("a.rs", "@@ -0,0 +1 @@\n+y")]),
    ]);
    src.diffs.get_mut("c2").unwrap().base_sha = "c0".into();
    src.direct.insert(
        ("c0".into(), "c2".into()),
        diff(
            "c0",
            "c2",
            vec![changed(
                "a.rs",
                "@@ -0,0 +1,2 @@\n+x\n+y\n@@ -5 +7 @@\n-f\n+F",
            )],
        ),
    );
    let m = walk_line(&src, &start("a.rs", 5, 5), "c2").unwrap();
    assert_eq!(where_(&m), located("a.rs", 7, 7));
    assert!(!m.attributed);
    assert!(m.touched_by.is_empty());
    assert!(matches!(
        m.status,
        LineStatus::Located { touched: true, .. }
    ));

    // The comment's commit was rebased away, but the platform still diffs
    // it straight against head.
    src.direct.insert(
        ("old".into(), "c2".into()),
        diff("old", "c2", vec![changed("a.rs", "@@ -0,0 +1 @@\n+z")]),
    );
    let m = walk_line(
        &src,
        &WalkStart {
            sha: "old".into(),
            ..start("a.rs", 5, 5)
        },
        "c2",
    )
    .unwrap();
    assert_eq!(where_(&m), located("a.rs", 6, 6));
}

#[test]
fn a_commit_that_names_no_parent_is_not_taken_as_linear() {
    // c2 names no parent (GitLab's commit diff did not): it may merge a
    // side branch, whose changes c2's own diff repeats, so walking commit by
    // commit would count the insertion twice. The direct diff decides.
    let mut src = MemSource::new(vec![
        ("c1", vec![changed("a.rs", "@@ -0,0 +1 @@\n+x")]),
        ("c2", vec![changed("a.rs", "@@ -0,0 +1 @@\n+x")]),
    ]);
    src.diffs.get_mut("c2").unwrap().base_sha = String::new();
    src.direct.insert(
        ("c0".into(), "c2".into()),
        diff("c0", "c2", vec![changed("a.rs", "@@ -0,0 +1 @@\n+x")]),
    );
    let m = walk_line(&src, &start("a.rs", 5, 5), "c2").unwrap();
    assert_eq!(where_(&m), located("a.rs", 6, 6));
    assert!(!m.attributed);
}

// -------------------------------------------------------------- snapshots

fn file_of(n: u32) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

#[test]
fn windows_clamp_at_the_start_and_end_of_the_file() {
    let text = file_of(30);
    let w = window("s", "a.rs", &text, 2, 2).unwrap();
    assert_eq!((w.first_line, w.lines.len()), (1, 12));
    assert_eq!(w.lines[0], "line 1");
    let w = window("s", "a.rs", &text, 15, 17).unwrap();
    assert_eq!((w.first_line, w.lines.len()), (5, 23));
    assert_eq!((w.anchor_start, w.anchor_end), (Some(15), Some(17)));
    let w = window("s", "a.rs", &text, 30, 30).unwrap();
    assert_eq!((w.first_line, w.lines.len()), (20, 11));
    assert_eq!(w.lines.last().unwrap(), "line 30");
    // Past the end: not where it was said to be.
    assert!(window("s", "a.rs", &text, 31, 31).is_none());
    assert!(window("s", "a.rs", "", 1, 1).is_none());
    assert!(window("s", "a.rs", &text, 0, 0).is_none());

    let a = around("s", "a.rs", &text, 0).unwrap();
    assert_eq!((a.first_line, a.lines.len(), a.anchor_start), (1, 10, None));
    let a = around("s", "a.rs", &text, 15).unwrap();
    assert_eq!((a.first_line, a.lines.len()), (6, 20));
    let a = around("s", "a.rs", &text, 30).unwrap();
    assert_eq!((a.first_line, a.lines.len()), (21, 10));
}

#[test]
fn a_diff_hunk_stands_in_for_the_then_file() {
    let hunk = "@@ -10,5 +10,6 @@ impl Cache {\n     fn get() {\n-        old();\n+        new();\n+        more();\n     }\n }";
    let s = from_diff_hunk("s", "a.rs", hunk, Side::New, 12, 12).unwrap();
    assert_eq!(s.source, SnapshotSource::DiffHunk);
    assert_eq!(s.first_line, 10);
    assert_eq!(s.lines[2], "        more();");
    assert_eq!(s.lines.len(), 5);
    let old = from_diff_hunk("s", "a.rs", hunk, Side::Old, 11, 11).unwrap();
    assert_eq!(old.lines[1], "        old();");
    // The commented line is not in the hunk.
    assert!(from_diff_hunk("s", "a.rs", hunk, Side::New, 40, 40).is_none());
}

#[test]
fn diff_rows_pair_changes_side_by_side() {
    let then = CodeSnapshot {
        sha: "a".into(),
        path: "x".into(),
        first_line: 9,
        lines: vec!["a".into(), "b".into(), "c".into(), "d".into()],
        anchor_start: Some(10),
        anchor_end: Some(10),
        source: SnapshotSource::File,
    };
    let now = CodeSnapshot {
        first_line: 19,
        lines: vec!["a".into(), "B".into(), "c".into(), "new".into(), "d".into()],
        anchor_start: Some(20),
        anchor_end: Some(20),
        ..then.clone()
    };
    let rows = diff_rows(&then, &now);
    let kinds: Vec<DiffKind> = rows.iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        [
            DiffKind::Same,
            DiffKind::Changed,
            DiffKind::Same,
            DiffKind::Added,
            DiffKind::Same
        ]
    );
    let changed = &rows[1];
    let (t, n) = (
        changed.then.as_ref().unwrap(),
        changed.now.as_ref().unwrap(),
    );
    assert_eq!((t.line, t.text.as_str(), t.anchored), (10, "b", true));
    assert_eq!((n.line, n.text.as_str(), n.anchored), (20, "B", true));
    assert!(rows[3].then.is_none());
    assert!(!rows[0].then.as_ref().unwrap().anchored);
    let json = serde_json::to_value(&rows[1]).unwrap();
    assert_eq!(json["kind"], "changed");
    assert_eq!(json["now"]["anchored"], true);
}

// --------------------------------------------------------------- timeline

#[test]
fn timeline_sorts_by_time_with_stable_ties_and_undated_last() {
    let entry = |at: Option<chrono::DateTime<chrono::Utc>>, by: &str| TimelineEntry {
        at,
        event: TimelineEvent::Resolved { by: by.into() },
    };
    let merged = merge(vec![
        entry(Some(at(10, 12)), "b"),
        entry(None, "undated"),
        entry(Some(at(10, 9)), "a"),
        entry(Some(at(10, 12)), "c"),
    ]);
    let order: Vec<&str> = merged
        .iter()
        .map(|e| match &e.event {
            TimelineEvent::Resolved { by } => by.as_str(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(order, ["a", "b", "c", "undated"]);
    let json = serde_json::to_value(&merged[0]).unwrap();
    assert_eq!(json["kind"], "resolved");
    assert!(json["at"].is_string());
}

#[test]
fn line_touch_answers_relative_to_the_watermark() {
    let lt = LineTouch {
        then_sha: "c1".into(),
        commits: ["c1", "c2", "c3", "c4"].map(String::from).to_vec(),
        touched_by: vec!["c3".into()],
        usable: true,
    };
    assert_eq!(lt.touched_since(None), Some(true));
    assert_eq!(lt.touched_since(Some("c1")), Some(true));
    assert_eq!(lt.touched_since(Some("c2")), Some(true));
    assert_eq!(lt.touched_since(Some("c3")), Some(false));
    assert_eq!(lt.touched_since(Some("c4")), Some(false));
    // A watermark the walk does not know (rebased away): cannot tell.
    assert_eq!(lt.touched_since(Some("zz")), None);
    let quiet = LineTouch {
        touched_by: Vec::new(),
        ..lt.clone()
    };
    assert_eq!(quiet.touched_since(None), Some(false));
    let unusable = LineTouch {
        usable: false,
        ..lt
    };
    assert_eq!(unusable.touched_since(None), None);
}

#[test]
fn statuses_serialize_tagged_camel_case() {
    let v = serde_json::to_value(LineStatus::FileGone {
        path: "a".into(),
        removed_by: Some("c1".into()),
    })
    .unwrap();
    assert_eq!(v["kind"], "fileGone");
    assert_eq!(v["removedBy"], "c1");
    let v = serde_json::to_value(LineStatus::Located {
        path: "a".into(),
        start_line: 1,
        line: 2,
        moved: false,
        renamed: false,
        touched: true,
    })
    .unwrap();
    assert_eq!(v["startLine"], 1);
    assert_eq!(
        serde_json::to_value(LineStatus::NotCode).unwrap()["kind"],
        "notCode"
    );
    let v = serde_json::to_value(TimelineEntry {
        at: None,
        event: TimelineEvent::Commit {
            sha: "c".into(),
            title: "t".into(),
            author: "a".into(),
            change: LineChange::Renamed {
                from: "x".into(),
                to: "y".into(),
            },
        },
    })
    .unwrap();
    assert_eq!(v["kind"], "commit");
    assert_eq!(v["change"]["kind"], "renamed");
    assert_eq!(v["at"], serde_json::Value::Null);
}

// ------------------------------------------------------------------ cache

fn cached(status: LineStatus, entries: Vec<TimelineEntry>) -> CachedHistory {
    CachedHistory {
        mapping: CachedMapping {
            status,
            touched_by: vec!["c2".into()],
            attributed: true,
            then_sha: Some("c1".into()),
            commits: vec!["c1".into(), "c2".into()],
            resolved: false,
        },
        then_snapshot: window("c1", "a.rs", &file_of(5), 2, 2),
        now_snapshot: None,
        entries,
    }
}

#[test]
fn the_cache_is_keyed_by_thread_and_head() {
    let store = Store::in_memory().unwrap();
    let rec = crate::review::advance(None, &detail("h1"), &[], ME, true, at(10, 10));
    store.upsert_review(&rec).unwrap();
    let threads = [
        thread("1", ME, "c1", vec![note("1", ME, at(10, 11))]),
        thread("2", ME, "c1", vec![note("2", ME, at(10, 11))]),
    ];
    store.upsert_threads(rec.id, &threads, ME).unwrap();
    let t1 = crate::ids::ThreadId::from_key(rec.id, "1");
    let t2 = crate::ids::ThreadId::from_key(rec.id, "2");
    assert_eq!(store.thread_review(t1).unwrap(), Some(rec.id));

    let entry = TimelineEntry {
        at: Some(at(10, 12)),
        event: TimelineEvent::Commit {
            sha: "c2".into(),
            title: "Fix".into(),
            author: AUTHOR.into(),
            change: LineChange::Modified,
        },
    };
    let h = cached(
        LineStatus::Located {
            path: "a.rs".into(),
            start_line: 2,
            line: 2,
            moved: false,
            renamed: false,
            touched: true,
        },
        vec![entry.clone()],
    );
    store.put_history(t1, "h1", &h).unwrap();
    assert_eq!(store.cached_history(t1, "h1").unwrap(), Some(h.clone()));
    // Another head, another thread: misses.
    assert_eq!(store.cached_history(t1, "h2").unwrap(), None);
    assert_eq!(store.cached_history(t2, "h1").unwrap(), None);
    let touches = store.line_touches(rec.id, "h1").unwrap();
    assert_eq!(touches.len(), 1);
    assert_eq!(touches["1"].touched_by, ["c2"]);
    assert!(touches["1"].usable);
    assert!(store.line_touches(rec.id, "h2").unwrap().is_empty());

    // The head moved: the new entry replaces the old one.
    let h2 = cached(LineStatus::unlocated("x"), Vec::new());
    store.put_history(t1, "h2", &h2).unwrap();
    assert_eq!(store.cached_history(t1, "h1").unwrap(), None);
    assert_eq!(store.cached_history(t1, "h2").unwrap(), Some(h2));
    assert!(!store.line_touches(rec.id, "h2").unwrap()["1"].usable);

    // The thread is gone on the platform: so is its cache.
    store.upsert_threads(rec.id, &threads[1..], ME).unwrap();
    assert_eq!(store.cached_history(t1, "h2").unwrap(), None);
}

// ------------------------------------------------- the core, end to end

/// A provider over an in-memory repository: files by (sha, path), commit
/// diffs by sha, and counters for what was fetched.
struct Repo {
    detail: Mutex<ReviewDetail>,
    threads: Mutex<Vec<Thread>>,
    files: Mutex<HashMap<(String, String), String>>,
    diffs: Mutex<HashMap<String, FileDiff>>,
    resolutions: Mutex<Vec<ThreadResolution>>,
    file_calls: AtomicUsize,
    diff_calls: AtomicUsize,
    fail_files: Mutex<bool>,
}

impl ReviewProvider for Repo {
    fn id(&self) -> ProviderId {
        ProviderId::Github
    }
    fn host(&self) -> &str {
        "github.com"
    }
    fn current_user(&self) -> CoreResult<String> {
        Ok(ME.into())
    }
    fn list_reviews(&self) -> CoreResult<Vec<ReviewRef>> {
        Ok(vec![self.detail.lock().unwrap().review.clone()])
    }
    fn fetch_review(&self, _: &ReviewRef) -> CoreResult<ReviewDetail> {
        Ok(self.detail.lock().unwrap().clone())
    }
    fn fetch_threads(&self, _: &ReviewRef) -> CoreResult<Vec<Thread>> {
        Ok(self.threads.lock().unwrap().clone())
    }
    fn file_at(&self, _: &ReviewRef, sha: &str, path: &str) -> CoreResult<String> {
        self.file_calls.fetch_add(1, Ordering::SeqCst);
        if *self.fail_files.lock().unwrap() {
            return Err(CoreError::Provider("offline".into()));
        }
        self.files
            .lock()
            .unwrap()
            .get(&(sha.to_string(), path.to_string()))
            .cloned()
            .ok_or(CoreError::NotFound)
    }
    fn fetch_files(&self, _: &ReviewRef, _: &DiffRange) -> CoreResult<FileDiff> {
        Err(CoreError::NotFound)
    }
    fn commit_diff(&self, _: &ReviewRef, sha: &str) -> CoreResult<FileDiff> {
        self.diff_calls.fetch_add(1, Ordering::SeqCst);
        self.diffs
            .lock()
            .unwrap()
            .get(sha)
            .cloned()
            .ok_or(CoreError::NotFound)
    }
    fn thread_resolutions(&self, _: &ReviewRef) -> CoreResult<Vec<ThreadResolution>> {
        Ok(self.resolutions.lock().unwrap().clone())
    }
}

fn commit(sha: &str, day: u32, hour: u32) -> CommitInfo {
    CommitInfo {
        sha: sha.into(),
        title: format!("Commit {sha}"),
        author: AUTHOR.into(),
        authored_at: Some(at(day, hour)),
    }
}

/// c1: the file; the user comments on line 10 at c1 (day 10, 11:00). c2
/// (day 11) inserts two lines at the top; c3 (day 12) rewrites the line.
fn repo() -> Arc<Repo> {
    let v1 = file_of(30);
    let v2 = format!("x\ny\n{v1}");
    let v3 = v2.replace("line 10\n", "line ten\n");
    let mut d = detail("c3");
    d.commits = vec![
        commit("c1", 10, 9),
        commit("c2", 11, 9),
        commit("c3", 12, 9),
    ];
    let files = HashMap::from([
        (("c1".into(), "src/cache.rs".into()), v1),
        (("c2".into(), "src/cache.rs".into()), v2),
        (("c3".into(), "src/cache.rs".into()), v3),
    ]);
    let diffs = HashMap::from([
        (
            "c2".to_string(),
            diff(
                "c1",
                "c2",
                vec![changed("src/cache.rs", "@@ -0,0 +1,2 @@\n+x\n+y")],
            ),
        ),
        (
            "c3".to_string(),
            diff(
                "c2",
                "c3",
                vec![changed(
                    "src/cache.rs",
                    "@@ -12 +12 @@\n-line 10\n+line ten",
                )],
            ),
        ),
    ]);
    let mine = thread("100", ME, "c1", vec![note("n1", ME, at(10, 11))]);
    let mut general = thread("200", AUTHOR, "c1", vec![note("n2", AUTHOR, at(10, 12))]);
    general.anchor = None;
    Arc::new(Repo {
        detail: Mutex::new(d),
        threads: Mutex::new(vec![mine, general]),
        files: Mutex::new(files),
        diffs: Mutex::new(diffs),
        resolutions: Mutex::new(Vec::new()),
        file_calls: AtomicUsize::new(0),
        diff_calls: AtomicUsize::new(0),
        fail_files: Mutex::new(false),
    })
}

fn core_over(repo: &Arc<Repo>) -> Core {
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![repo.clone()]);
    let report = core.refresh();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    core
}

#[test]
fn computes_then_now_and_the_timeline() {
    let repo = repo();
    let core = core_over(&repo);
    let id = core.reviews().unwrap()[0].id;
    let all = core.review_histories(id).unwrap();
    assert_eq!(all.len(), 2);
    let h = &all[0];
    assert_eq!(h.head_sha, "c3");
    assert_eq!(
        h.status,
        LineStatus::Located {
            path: "src/cache.rs".into(),
            start_line: 12,
            line: 12,
            moved: true,
            renamed: false,
            touched: true
        }
    );
    assert_eq!(h.touched_by, ["c3"]);
    let then = h.then_snapshot.as_ref().unwrap();
    assert_eq!((then.sha.as_str(), then.anchor_end), ("c1", Some(10)));
    let now = h.now_snapshot.as_ref().unwrap();
    assert_eq!((now.sha.as_str(), now.anchor_end), ("c3", Some(12)));
    let changed: Vec<_> = h
        .diff_lines
        .iter()
        .filter(|r| r.kind == DiffKind::Changed)
        .collect();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].now.as_ref().unwrap().text, "line ten");
    assert!(changed[0].then.as_ref().unwrap().anchored);
    // The comment, then the commit that rewrote the line; c2 only moved it.
    let kinds: Vec<String> = h
        .timeline
        .iter()
        .map(|e| {
            serde_json::to_value(e).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(kinds, ["note", "commit"]);
    assert!(matches!(
        &h.timeline[1].event,
        TimelineEvent::Commit { sha, change: LineChange::Modified, .. } if sha == "c3"
    ));

    // The MR-level thread: notes only.
    let g = &all[1];
    assert_eq!(g.status, LineStatus::NotCode);
    assert!(g.then_snapshot.is_none() && g.diff_lines.is_empty());
    assert_eq!(g.timeline.len(), 1);

    // The flat cached text for the AI prompt: no provider call.
    let calls = repo.file_calls.load(Ordering::SeqCst);
    let text = core.cached_snapshot(h.thread_id).unwrap().unwrap();
    assert_eq!(text.now_line, Some(12));
    assert_eq!(text.now_path.as_deref(), Some("src/cache.rs"));
    assert!(text.then_code.unwrap().contains("line 10"));
    assert!(text.now_code.unwrap().contains("line ten"));
    assert_eq!(repo.file_calls.load(Ordering::SeqCst), calls);
    assert_eq!(
        core.cached_snapshot(crate::ids::ThreadId::new()).unwrap(),
        None
    );

    let json = serde_json::to_value(h).unwrap();
    for key in [
        "threadId",
        "status",
        "thenSnapshot",
        "nowSnapshot",
        "diffLines",
        "timeline",
        "touchedBy",
    ] {
        assert!(json.get(key).is_some(), "missing {key}");
    }
}

#[test]
fn cached_until_the_head_moves() {
    let repo = repo();
    let core = core_over(&repo);
    let id = core.reviews().unwrap()[0].id;
    let first = core.review_histories(id).unwrap();
    let (files, diffs) = (
        repo.file_calls.load(Ordering::SeqCst),
        repo.diff_calls.load(Ordering::SeqCst),
    );
    assert_eq!((files, diffs), (2, 2));

    // A reply lands without new commits: in the timeline, no refetch.
    let mut threads = repo.threads.lock().unwrap().clone();
    threads[0].notes.push(note("n3", AUTHOR, at(12, 10)));
    *repo.threads.lock().unwrap() = threads;
    core.refresh();
    let thread_id = first[0].thread_id;
    let again = core.thread_history(thread_id).unwrap();
    assert_eq!(again.status, first[0].status);
    assert_eq!(again.timeline.len(), 3);
    assert_eq!(repo.file_calls.load(Ordering::SeqCst), files);
    assert_eq!(repo.diff_calls.load(Ordering::SeqCst), diffs);

    // The head moves to c4, which does not touch the file: recomputed.
    {
        let mut d = repo.detail.lock().unwrap();
        d.head_sha = "c4".into();
        d.commits.push(commit("c4", 13, 9));
    }
    let v3 = repo.files.lock().unwrap()[&("c3".to_string(), "src/cache.rs".to_string())].clone();
    repo.files
        .lock()
        .unwrap()
        .insert(("c4".into(), "src/cache.rs".into()), v3);
    repo.diffs.lock().unwrap().insert(
        "c4".into(),
        diff(
            "c3",
            "c4",
            vec![changed("README.md", "@@ -1 +1 @@\n-a\n+b")],
        ),
    );
    core.refresh();
    let moved = core.thread_history(thread_id).unwrap();
    assert_eq!(moved.head_sha, "c4");
    assert_eq!(moved.touched_by, ["c3"]);
    assert!(repo.diff_calls.load(Ordering::SeqCst) > diffs);
}

#[test]
fn provider_failures_degrade_and_are_retried() {
    let repo = repo();
    let core = core_over(&repo);
    let id = core.reviews().unwrap()[0].id;
    *repo.fail_files.lock().unwrap() = true;
    let h = &core.review_histories(id).unwrap()[0];
    // The walk itself worked; only the snapshots are missing.
    assert!(matches!(h.status, LineStatus::Located { .. }));
    assert!(h.then_snapshot.is_none() && h.now_snapshot.is_none());
    // Not cached: the next call fetches again, and now gets the files.
    *repo.fail_files.lock().unwrap() = false;
    let h = &core.review_histories(id).unwrap()[0];
    assert!(h.then_snapshot.is_some() && h.now_snapshot.is_some());
}

#[test]
fn an_untouched_line_that_reads_differently_is_unlocated() {
    let repo = repo();
    // c2's diff claims only an insertion, but the file says otherwise.
    let wrong = file_of(32).replace("line 12\n", "something else\n");
    repo.files
        .lock()
        .unwrap()
        .insert(("c2".into(), "src/cache.rs".into()), wrong);
    {
        let mut d = repo.detail.lock().unwrap();
        d.head_sha = "c2".into();
        d.commits.truncate(2);
    }
    let core = core_over(&repo);
    let id = core.reviews().unwrap()[0].id;
    let h = &core.review_histories(id).unwrap()[0];
    assert!(matches!(&h.status, LineStatus::Unlocated { reason } if reason.contains("reliably")));
    assert!(h.now_snapshot.is_none());
}

#[test]
fn resolve_events_join_the_timeline_and_refresh_the_cache() {
    let repo = repo();
    let core = core_over(&repo);
    let id = core.reviews().unwrap()[0].id;
    let before = core.review_histories(id).unwrap();
    assert_eq!(before[0].timeline.len(), 2);

    // Resolved on the platform: the cached entry is stale.
    let mut threads = repo.threads.lock().unwrap().clone();
    threads[0].resolved = true;
    *repo.threads.lock().unwrap() = threads;
    *repo.resolutions.lock().unwrap() = vec![ThreadResolution {
        thread_id: "100".into(),
        resolved: true,
        by: AUTHOR.into(),
        at: at(12, 11),
    }];
    core.refresh();
    let after = core.thread_history(before[0].thread_id).unwrap();
    assert_eq!(after.timeline.len(), 3);
    assert!(matches!(
        &after.timeline[2].event,
        TimelineEvent::Resolved { by } if by == AUTHOR
    ));
}

#[test]
fn the_walk_sharpens_the_untouched_count() {
    let repo = repo();
    // Head at c2: new commits since the comment, but none touched line 10.
    {
        let mut d = repo.detail.lock().unwrap();
        d.head_sha = "c2".into();
        d.commits.truncate(2);
    }
    let core = core_over(&repo);
    let rec = core.reviews().unwrap()[0].clone();
    // The approximation: c2 is a new commit, so the thread counts touched.
    assert_eq!(rec.untouched_threads, 0);
    core.review_histories(rec.id).unwrap();
    assert_eq!(core.review(rec.id).unwrap().untouched_threads, 1);
    // A later refresh keeps the walk's answer.
    let mut threads = repo.threads.lock().unwrap().clone();
    threads[1].notes.push(note("n9", AUTHOR, at(12, 9)));
    *repo.threads.lock().unwrap() = threads;
    assert!(!core.refresh().changed.is_empty());
    assert_eq!(core.review(rec.id).unwrap().untouched_threads, 1);

    // At c3 the line was rewritten: touched.
    {
        let mut d = repo.detail.lock().unwrap();
        d.head_sha = "c3".into();
        d.commits.push(commit("c3", 12, 9));
    }
    core.refresh();
    core.review_histories(rec.id).unwrap();
    assert_eq!(core.review(rec.id).unwrap().untouched_threads, 0);
}

#[test]
fn unknown_threads_are_not_found() {
    let core = Core::new();
    assert!(matches!(
        core.thread_history(crate::ids::ThreadId::new()),
        Err(CoreError::NotFound)
    ));
    assert!(matches!(
        core.review_histories(crate::ids::ReviewId::new()),
        Err(CoreError::NotFound)
    ));
    assert!(matches!(
        core.thread_review(crate::ids::ThreadId::new()),
        Err(CoreError::NotFound)
    ));
}

#[test]
fn a_stored_thread_knows_its_review() {
    let repo = repo();
    let core = core_over(&repo);
    let rec = core.reviews().unwrap()[0].clone();
    for t in core.threads(rec.id).unwrap() {
        assert_eq!(core.thread_review(t.thread_id).unwrap(), rec.id);
    }
}

#[test]
fn a_history_call_over_a_stale_record_keeps_the_round_and_the_watermark() {
    let repo = repo();
    // Head at c2: the walk turns the untouched count from 0 to 1.
    {
        let mut d = repo.detail.lock().unwrap();
        d.head_sha = "c2".into();
        d.commits.truncate(2);
    }
    let core = core_over(&repo);
    let stale = core.reviews().unwrap()[0].clone();
    assert_eq!(stale.status, ReviewStatus::NeedsReview);
    // The user marks it reviewed while a history call holds the old record.
    let marked = core.mark_reviewed(stale.id).unwrap();
    assert_eq!(marked.status, ReviewStatus::WaitingOnAuthor);

    let provider: CoreResult<Arc<dyn ReviewProvider>> = Ok(repo.clone());
    super::api::compute_histories(&core.store, &stale, &provider, None, None).unwrap();
    let rec = core.review(stale.id).unwrap();
    assert_eq!(rec.status, ReviewStatus::WaitingOnAuthor);
    assert_eq!(rec.last_reviewed_sha.as_deref(), Some("c2"));
    assert_eq!(rec.round, marked.round);
    // Counted against the stored watermark (at head: nothing touched since).
    assert_eq!(rec.untouched_threads, 1);

    // Walks for a head the review no longer has never write the count.
    let mut old = rec.clone();
    old.head_sha = "c1".into();
    old.untouched_threads = 7;
    core.store
        .set_round(rec.id, rec.round, &rec.round_reasons, rec.status, 0)
        .unwrap();
    super::api::compute_histories(&core.store, &old, &provider, None, None).unwrap();
    assert_eq!(core.review(rec.id).unwrap().untouched_threads, 0);
}

/// [`Repo`], but the review's threads vanish from the store when the
/// computation fetches the review: a poll that dropped them meanwhile.
struct Vanishing {
    repo: Arc<Repo>,
    store: Arc<Store>,
    review: crate::ids::ReviewId,
}

impl ReviewProvider for Vanishing {
    fn id(&self) -> ProviderId {
        self.repo.id()
    }
    fn host(&self) -> &str {
        "github.com"
    }
    fn current_user(&self) -> CoreResult<String> {
        self.repo.current_user()
    }
    fn list_reviews(&self) -> CoreResult<Vec<ReviewRef>> {
        self.repo.list_reviews()
    }
    fn fetch_review(&self, r: &ReviewRef) -> CoreResult<ReviewDetail> {
        self.store.upsert_threads(self.review, &[], ME).unwrap();
        self.repo.fetch_review(r)
    }
    fn fetch_threads(&self, r: &ReviewRef) -> CoreResult<Vec<Thread>> {
        self.repo.fetch_threads(r)
    }
    fn file_at(&self, r: &ReviewRef, sha: &str, path: &str) -> CoreResult<String> {
        self.repo.file_at(r, sha, path)
    }
    fn fetch_files(&self, r: &ReviewRef, range: &DiffRange) -> CoreResult<FileDiff> {
        self.repo.fetch_files(r, range)
    }
    fn commit_diff(&self, r: &ReviewRef, sha: &str) -> CoreResult<FileDiff> {
        self.repo.commit_diff(r, sha)
    }
    fn thread_resolutions(&self, r: &ReviewRef) -> CoreResult<Vec<ThreadResolution>> {
        self.repo.thread_resolutions(r)
    }
}

#[test]
fn a_thread_dropped_while_computing_is_answered_not_cached() {
    let repo = repo();
    let core = core_over(&repo);
    let record = core.reviews().unwrap()[0].clone();
    let provider: CoreResult<Arc<dyn ReviewProvider>> = Ok(Arc::new(Vanishing {
        repo: repo.clone(),
        store: core.store.clone(),
        review: record.id,
    }));
    let histories =
        super::api::compute_histories(&core.store, &record, &provider, None, None).unwrap();
    assert_eq!(histories.len(), 2);
    assert!(matches!(histories[0].status, LineStatus::Located { .. }));
    for h in &histories {
        assert_eq!(core.store.cached_history(h.thread_id, "c3").unwrap(), None);
    }
}

#[test]
fn caching_for_a_thread_that_is_gone_stores_nothing() {
    let store = Store::in_memory().unwrap();
    let gone = crate::ids::ThreadId::new();
    let h = cached(LineStatus::unlocated("x"), Vec::new());
    store.put_history(gone, "h1", &h).unwrap();
    assert_eq!(store.cached_history(gone, "h1").unwrap(), None);
}
