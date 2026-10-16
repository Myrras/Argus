use std::sync::{Arc, Mutex};

use super::*;
use crate::provider::{FileStatus, ProviderId, ReviewDetail, ReviewProvider, ReviewRef, Thread};
use crate::store::Store;
use crate::testutil::{detail, ME};
use crate::Core;

fn file(path: &str, sha: &str) -> ChangedFile {
    ChangedFile {
        path: path.into(),
        old_path: None,
        status: FileStatus::Modified,
        additions: 1,
        deletions: 1,
        patch: Some("@@ -1 +1 @@\n-a\n+b".into()),
        content_sha: sha.into(),
        binary: false,
        too_large: false,
    }
}

/// Serves one review whose head and files a test can move, and records the
/// ranges it was asked for.
struct Fake {
    detail: Mutex<ReviewDetail>,
    review_files: Mutex<Vec<ChangedFile>>,
    compare_files: Mutex<Vec<ChangedFile>>,
    asked: Mutex<Vec<DiffRange>>,
}

impl ReviewProvider for Fake {
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
        Ok(Vec::new())
    }
    fn file_at(&self, _: &ReviewRef, _: &str, _: &str) -> CoreResult<String> {
        Err(CoreError::NotFound)
    }
    fn fetch_files(&self, _: &ReviewRef, range: &DiffRange) -> CoreResult<FileDiff> {
        self.asked.lock().unwrap().push(range.clone());
        let head = self.detail.lock().unwrap().head_sha.clone();
        Ok(match range {
            DiffRange::Review { .. } => FileDiff {
                base_sha: "mergebase".into(),
                head_sha: head,
                files: self.review_files.lock().unwrap().clone(),
                truncated: false,
            },
            DiffRange::Compare { from, .. } => FileDiff {
                base_sha: from.clone(),
                head_sha: head,
                files: self.compare_files.lock().unwrap().clone(),
                truncated: false,
            },
        })
    }
}

fn setup() -> (Core, Arc<Fake>, ReviewId) {
    let fake = Arc::new(Fake {
        detail: Mutex::new(detail("h1")),
        review_files: Mutex::new(vec![file("a.rs", "a1"), file("b.rs", "b1")]),
        compare_files: Mutex::new(Vec::new()),
        asked: Mutex::new(Vec::new()),
    });
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![fake.clone()]);
    assert!(core.refresh().errors.is_empty());
    let id = core.reviews().unwrap()[0].id;
    (core, fake, id)
}

fn viewed(files: &ReviewFiles) -> Vec<&str> {
    files
        .files
        .iter()
        .filter(|f| f.viewed)
        .map(|f| f.file.path.as_str())
        .collect()
}

#[test]
fn viewed_marks_persist_and_reset_when_the_file_changes() {
    let (core, fake, id) = setup();
    let files = core.list_files(id, false).unwrap();
    assert_eq!(files.base_sha, "mergebase");
    assert_eq!(files.head_sha, "h1");
    assert!(!files.since_last_review);
    assert!(viewed(&files).is_empty());

    assert_eq!(core.set_file_viewed(id, "a.rs", true).unwrap(), ["a.rs"]);
    assert_eq!(
        core.set_file_viewed(id, "b.rs", true).unwrap(),
        ["a.rs", "b.rs"]
    );
    assert_eq!(
        viewed(&core.list_files(id, false).unwrap()),
        ["a.rs", "b.rs"]
    );
    assert_eq!(core.set_file_viewed(id, "b.rs", false).unwrap(), ["a.rs"]);
    assert!(matches!(
        core.set_file_viewed(id, "nope.rs", true),
        Err(CoreError::InvalidInput(_))
    ));

    // A push changes a.rs: its mark no longer counts, b.rs is untouched.
    *fake.review_files.lock().unwrap() = vec![file("a.rs", "a2"), file("b.rs", "b1")];
    fake.detail.lock().unwrap().head_sha = "h2".into();
    core.refresh();
    assert!(viewed(&core.list_files(id, false).unwrap()).is_empty());
    // Marking again (the keys are refetched for the new head) sticks.
    assert_eq!(core.set_file_viewed(id, "a.rs", true).unwrap(), ["a.rs"]);
}

#[test]
fn since_last_review_diffs_from_the_watermark_with_the_review_keys() {
    let (core, fake, id) = setup();
    assert!(matches!(
        core.list_files(id, true),
        Err(CoreError::InvalidInput(_))
    ));
    core.mark_reviewed(id).unwrap();
    // Marked at the head: still nothing to compare.
    assert!(core.list_files(id, true).is_err());

    core.set_file_viewed(id, "b.rs", true).unwrap();
    fake.detail.lock().unwrap().head_sha = "h2".into();
    // The compare's own sha for b.rs differs (GitLab digests the compare's
    // diff); the mark still counts because keys come from the review diff.
    *fake.compare_files.lock().unwrap() = vec![file("b.rs", "other-digest")];
    core.refresh();
    fake.asked.lock().unwrap().clear();

    let files = core.list_files(id, true).unwrap();
    assert!(files.since_last_review);
    assert_eq!(files.base_sha, "h1");
    assert_eq!(files.files.len(), 1);
    assert_eq!(viewed(&files), ["b.rs"]);
    let asked = fake.asked.lock().unwrap().clone();
    assert!(asked.contains(&DiffRange::Compare {
        from: "h1".into(),
        to: "h2".into()
    }));
    assert!(asked
        .iter()
        .any(|r| matches!(r, DiffRange::Review { head, .. } if head == "h2")));
}

#[test]
fn serializes_flat_camel_case() {
    let entry = FileEntry {
        file: ChangedFile {
            old_path: Some("old.rs".into()),
            status: FileStatus::Renamed,
            ..file("new.rs", "s")
        },
        viewed: true,
    };
    let v = serde_json::to_value(&entry).unwrap();
    assert_eq!(v["path"], "new.rs");
    assert_eq!(v["oldPath"], "old.rs");
    assert_eq!(v["status"], "renamed");
    assert_eq!(v["contentSha"], "s");
    assert_eq!(v["tooLarge"], false);
    assert_eq!(v["viewed"], true);
}
