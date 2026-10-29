use super::*;
use crate::provider::{ChangedFile, FileStatus};
use crate::testutil::review_ref;

const A: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
const B: &str = "b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2";
const C: &str = "c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3";

fn cache(store: &Store, limit: u64) -> DiskCache<'_> {
    DiskCache { store, limit }
}

fn diff(base: &str, head: &str) -> FileDiff {
    FileDiff {
        base_sha: base.into(),
        head_sha: head.into(),
        files: vec![ChangedFile {
            path: "src/cache.rs".into(),
            old_path: None,
            status: FileStatus::Modified,
            additions: 1,
            deletions: 1,
            patch: Some("@@ -1 +1 @@\n-a\n+b".into()),
            content_sha: "blob1".into(),
            binary: false,
            too_large: false,
        }],
        truncated: false,
        base_approximate: false,
    }
}

#[test]
fn a_file_is_a_miss_until_stored_then_a_hit_at_that_commit_only() {
    let store = Store::in_memory().unwrap();
    let c = cache(&store, 1 << 20);
    let r = review_ref(1);
    assert_eq!(c.file(&r, A, "src/cache.rs"), None);
    c.put_file(&r, A, "src/cache.rs", "fn a() {}\n");
    assert_eq!(
        c.file(&r, A, "src/cache.rs").as_deref(),
        Some("fn a() {}\n")
    );
    // Another commit, path, repository or host is another key.
    assert_eq!(c.file(&r, B, "src/cache.rs"), None);
    assert_eq!(c.file(&r, A, "src/other.rs"), None);
    let mut other_repo = review_ref(1);
    other_repo.repo = "acme-fixture/other".into();
    assert_eq!(c.file(&other_repo, A, "src/cache.rs"), None);
    let mut other_host = review_ref(1);
    other_host.host = "gitlab.example.test".into();
    assert_eq!(c.file(&other_host, A, "src/cache.rs"), None);
}

#[test]
fn a_branch_name_or_short_sha_is_never_cached() {
    let store = Store::in_memory().unwrap();
    let c = cache(&store, 1 << 20);
    let r = review_ref(1);
    for sha in ["main", "a1a1a1a", "HEAD"] {
        c.put_file(&r, sha, "src/cache.rs", "moving target");
        assert_eq!(c.file(&r, sha, "src/cache.rs"), None, "{sha}");
    }
    assert_eq!(store.cache_stats().unwrap().entries, 0);
}

#[test]
fn a_file_list_is_cached_per_range_and_a_new_head_misses() {
    let store = Store::in_memory().unwrap();
    let c = cache(&store, 1 << 20);
    let r = review_ref(1);
    let at_b = DiffRange::Review {
        base: A.into(),
        head: B.into(),
    };
    c.put_diff(&r, &at_b, &diff(A, B));
    assert_eq!(c.diff(&r, &at_b), Some(diff(A, B)));
    let at_c = DiffRange::Review {
        base: A.into(),
        head: C.into(),
    };
    assert_eq!(c.diff(&r, &at_c), None);
    // A compare between the same commits is another list.
    let compare = DiffRange::Compare {
        from: A.into(),
        to: B.into(),
    };
    assert_eq!(c.diff(&r, &compare), None);
    // Another review of the repository too.
    assert_eq!(c.diff(&review_ref(2), &at_b), None);
}

#[test]
fn a_diff_of_a_newer_head_than_its_key_is_not_cached() {
    let store = Store::in_memory().unwrap();
    let c = cache(&store, 1 << 20);
    let r = review_ref(1);
    // The review moved since the poll: the platform answered for C.
    let stored = DiffRange::Review {
        base: A.into(),
        head: B.into(),
    };
    c.put_diff(&r, &stored, &diff(A, C));
    assert_eq!(c.diff(&r, &stored), None);
}

#[test]
fn eviction_drops_the_least_recently_used_down_to_the_target() {
    let store = Store::in_memory().unwrap();
    // Room for four 100-byte entries.
    let limit = 400;
    let c = cache(&store, limit);
    let r = review_ref(1);
    let body = "x".repeat(100);
    for path in ["one", "two", "three", "four"] {
        c.put_file(&r, A, path, &body);
    }
    assert_eq!(store.cache_stats().unwrap().bytes, 400);
    // Reading "one" makes "two" the least recently used.
    assert!(c.file(&r, A, "one").is_some());
    c.put_file(&r, A, "five", &body);
    let stats = store.cache_stats().unwrap();
    assert!(stats.bytes <= (limit as f64 * EVICT_TO) as u64, "{stats:?}");
    assert!(c.file(&r, A, "two").is_none());
    assert!(c.file(&r, A, "one").is_some());
    assert!(c.file(&r, A, "five").is_some());
}

#[test]
fn an_entry_larger_than_the_limit_is_not_stored() {
    let store = Store::in_memory().unwrap();
    let c = cache(&store, 10);
    c.put_file(&review_ref(1), A, "big", &"x".repeat(11));
    assert_eq!(store.cache_stats().unwrap().entries, 0);
}

#[test]
fn trimming_to_a_lower_limit_evicts_at_once() {
    let store = Store::in_memory().unwrap();
    let c = cache(&store, 1 << 20);
    let r = review_ref(1);
    for path in ["one", "two", "three"] {
        c.put_file(&r, A, path, &"x".repeat(100));
    }
    store.cache_trim(150).unwrap();
    let stats = store.cache_stats().unwrap();
    assert_eq!((stats.entries, stats.bytes), (1, 100));
    assert!(c.file(&r, A, "three").is_some());
}

#[test]
fn forgetting_a_reviews_lists_keeps_its_files_and_other_reviews() {
    let store = Store::in_memory().unwrap();
    let c = cache(&store, 1 << 20);
    let range = DiffRange::Review {
        base: A.into(),
        head: B.into(),
    };
    let compare = DiffRange::Compare {
        from: A.into(),
        to: B.into(),
    };
    c.put_diff(&review_ref(1), &range, &diff(A, B));
    c.put_diff(&review_ref(1), &compare, &diff(A, B));
    // Review 12 shares review 1's key prefix up to the number.
    c.put_diff(&review_ref(12), &range, &diff(A, B));
    c.put_file(&review_ref(1), A, "src/cache.rs", "text");
    store.cache_forget_diffs(&review_ref(1)).unwrap();
    assert_eq!(c.diff(&review_ref(1), &range), None);
    assert_eq!(c.diff(&review_ref(1), &compare), None);
    assert!(c.diff(&review_ref(12), &range).is_some());
    assert!(c.file(&review_ref(1), A, "src/cache.rs").is_some());
}

#[test]
fn clearing_empties_it() {
    let store = Store::in_memory().unwrap();
    let c = cache(&store, 1 << 20);
    c.put_file(&review_ref(1), A, "one", "text");
    store.cache_clear().unwrap();
    assert_eq!(
        store.cache_stats().unwrap(),
        CacheStats {
            entries: 0,
            bytes: 0
        }
    );
}

#[test]
fn the_limit_is_clamped() {
    assert_eq!(limit_bytes(0), MIN_LIMIT_MB * 1024 * 1024);
    assert_eq!(limit_bytes(512), 512 * 1024 * 1024);
    assert_eq!(limit_bytes(u64::MAX / 2), MAX_LIMIT_MB * 1024 * 1024);
}
