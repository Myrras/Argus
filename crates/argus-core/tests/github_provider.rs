//! The GitHub provider against a fake `gh` script that replays the fixtures
//! under `tests/fixtures/github/`. No network, no real token.
//!
//! The script is a POSIX shell script, so these tests only run on Unix; they
//! still compile on Windows.

// On Windows the fake is never built, so its helpers go unused.
#![cfg_attr(windows, allow(dead_code))]

use std::path::{Path, PathBuf};

use argus_core::provider::{
    DiffRange, FileStatus, GithubProvider, ProviderId, ReviewProvider, ReviewRef, ReviewState,
    ReviewerState, Side,
};
use argus_core::CoreError;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/github")
}

/// The fake: routes `gh` argument lines to fixture files and logs every call
/// (with the token env it saw) to `calls.log` next to it.
const FAKE: &str = r#"
echo "$* token=${GH_TOKEN:-none} ent=${GH_ENTERPRISE_TOKEN:-none} host=${GH_HOST:-none}" >> "$LOG"
case "$*" in
  "auth token --hostname github.com --user work") echo fake-gh-token ;;
  "auth token --hostname ghe.example.test --user work") echo fake-ghe-token ;;
  "auth token"*) echo "no account" >&2; exit 1 ;;
  "api user") cat "$F/user.json" ;;
  *"--review-requested=@me"*) cat "$F/search_review_requested.json" ;;
  *"--reviewed-by=@me"*) cat "$F/search_reviewed_by.json" ;;
  *"--commenter=@me"*) cat "$F/search_commenter.json" ;;
  "api repos/acme-fixture/widgets/pulls/12") cat "$F/pull_12.json" ;;
  "api --paginate repos/acme-fixture/widgets/pulls/12/commits?per_page=100")
    cat "$F/pull_12_commits_page1.json" "$F/pull_12_commits_page2.json" ;;
  "api --paginate repos/acme-fixture/widgets/pulls/12/reviews?per_page=100") cat "$F/pull_12_reviews.json" ;;
  "api --paginate repos/acme-fixture/widgets/pulls/12/comments?per_page=100")
    cat "$F/pull_12_comments_page1.json" "$F/pull_12_comments_page2.json" ;;
  "api --paginate repos/acme-fixture/widgets/pulls/12/files?per_page=100")
    cat "$F/pull_12_files_page1.json" "$F/pull_12_files_page2.json" ;;
  "api repos/acme-fixture/widgets/compare/b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0...c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3 --jq .merge_base_commit.sha")
    echo "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" ;;
  "api repos/acme-fixture/widgets/compare/c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1...c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3")
    cat "$F/compare_c1_c3.json" ;;
  "api repos/acme-fixture/gadgets/pulls/7") cat "$F/pull_7.json" ;;
  "api --paginate repos/acme-fixture/gadgets/pulls/7/commits?per_page=100") cat "$F/empty.json" ;;
  "api --paginate repos/acme-fixture/gadgets/pulls/7/reviews?per_page=100") cat "$F/pull_7_reviews.json" ;;
  "api repos/acme-fixture/widgets/contents/src/cache.rs?ref=c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3") cat "$F/contents_cache.json" ;;
  "api repos/acme-fixture/widgets/contents/big.bin?ref=c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3") cat "$F/contents_big.json" ;;
  *) echo "gh: Not Found (HTTP 404)" >&2; exit 1 ;;
esac
"#;

struct Fake {
    _dir: tempfile::TempDir,
    gh: String,
    log: PathBuf,
}

impl Fake {
    fn calls(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn provider(&self, host: &str, account: Option<&str>) -> GithubProvider {
        GithubProvider::with_binary(&self.gh, host, account.map(str::to_string))
    }
}

#[cfg(unix)]
fn script(body: &str) -> Fake {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("calls.log");
    let gh = dir.path().join("gh");
    let script = format!(
        "#!/bin/sh\nF='{}'\nLOG='{}'\n{body}\n",
        fixtures().display(),
        log.display()
    );
    std::fs::write(&gh, script).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    Fake {
        gh: gh.to_string_lossy().into_owned(),
        log,
        _dir: dir,
    }
}

#[cfg(not(unix))]
fn script(_body: &str) -> Fake {
    unreachable!("fake gh scripts are Unix-only; these tests are ignored on Windows")
}

fn widgets_12() -> ReviewRef {
    ReviewRef {
        provider: ProviderId::Github,
        host: "github.com".into(),
        repo: "acme-fixture/widgets".into(),
        number: 12,
        url: "https://github.com/acme-fixture/widgets/pull/12".into(),
        title: "Add widget caching".into(),
        author: "alice-fixture".into(),
        updated_at: None,
    }
}

#[test]
#[cfg_attr(windows, ignore)]
fn lists_merged_and_deduped_without_own_prs() {
    let fake = script(FAKE);
    let gh = fake.provider("github.com", None);
    assert_eq!(gh.current_user().unwrap(), "rev-fixture");
    let list = gh.list_reviews().unwrap();
    let keys: Vec<(String, u64)> = list.iter().map(|r| (r.repo.clone(), r.number)).collect();
    // Newest first; #12 appears in two searches but once here; #15 is the
    // user's own PR.
    assert_eq!(
        keys,
        [
            ("acme-fixture/gadgets".to_string(), 7),
            ("acme-fixture/widgets".to_string(), 12),
            ("acme-fixture/tools".to_string(), 3),
        ]
    );
    assert!(list.iter().all(|r| r.provider == ProviderId::Github));
    assert_eq!(list[1].author, "alice-fixture");
    let calls = fake.calls();
    assert!(calls.contains("search prs --review-requested=@me --state=open"));
    assert!(calls.contains("--commenter=@me"));
}

#[test]
#[cfg_attr(windows, ignore)]
fn fetches_detail_with_paginated_commits_and_reviewer_states() {
    let fake = script(FAKE);
    let gh = fake.provider("github.com", None);
    let d = gh.fetch_review(&widgets_12()).unwrap();
    assert_eq!(d.state, ReviewState::Open);
    assert_eq!(d.head_sha, "c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3");
    assert_eq!(d.base_sha, "b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0");
    assert_eq!(d.description, "Caches widgets for a minute.");
    let shas: Vec<&str> = d.commits.iter().map(|c| &c.sha[..2]).collect();
    assert_eq!(shas, ["c1", "c2", "c3"]);
    assert_eq!(d.commits[0].title, "Add a cache layer");
    // No GitHub login on the last commit: falls back to the git author name.
    assert_eq!(d.commits[2].author, "Alice Fixture");

    assert!(d.requested_from_me);
    assert!(!d.approved_by_me);
    let state_of = |u: &str| {
        d.reviewers
            .iter()
            .find(|r| r.username == u)
            .map(|r| r.state)
    };
    assert_eq!(state_of("rev-fixture"), Some(ReviewerState::Requested));
    // A later plain comment does not undo an approval.
    assert_eq!(state_of("dana-fixture"), Some(ReviewerState::Approved));
    // The author's replies are not reviews; a pending review is not one yet.
    assert_eq!(state_of("alice-fixture"), None);
    assert_eq!(state_of("erin-fixture"), None);
    assert!(d.reviewers.iter().any(|r| r.is_me));
}

#[test]
#[cfg_attr(windows, ignore)]
fn merged_pr_with_my_approval() {
    let fake = script(FAKE);
    let gh = fake.provider("github.com", None);
    let r = ReviewRef {
        repo: "acme-fixture/gadgets".into(),
        number: 7,
        ..widgets_12()
    };
    let d = gh.fetch_review(&r).unwrap();
    assert_eq!(d.state, ReviewState::Merged);
    assert!(d.approved_by_me);
    assert!(!d.requested_from_me);
    assert_eq!(d.review.author, "bob-fixture");
}

#[test]
#[cfg_attr(windows, ignore)]
fn rebuilds_threads_from_review_comments() {
    let fake = script(FAKE);
    let gh = fake.provider("github.com", None);
    let threads = gh.fetch_threads(&widgets_12()).unwrap();
    let ids: Vec<&str> = threads.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["501", "510", "530", "520"]);

    // Multi-reply thread across both pages; a reply to a reply still lands
    // on the root, in time order.
    let t = &threads[0];
    let notes: Vec<&str> = t.notes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(notes, ["501", "502", "503", "504"]);
    assert_eq!(t.author, "rev-fixture");
    assert!(!t.outdated);
    let a = t.anchor.as_ref().unwrap();
    assert_eq!(a.side, Side::New);
    assert_eq!(a.line_new, Some(10));
    assert_eq!(a.head_sha, "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1");
    assert!(a.diff_hunk.as_deref().unwrap().starts_with("@@"));

    assert_eq!(a.current_line, Some(10));

    // position == null → outdated, original line kept, no current line.
    let outdated = &threads[1];
    assert!(outdated.outdated);
    assert_eq!(outdated.anchor.as_ref().unwrap().line_new, Some(20));
    assert_eq!(outdated.anchor.as_ref().unwrap().current_line, None);

    // Multi-line: the original start line; lines added above moved it to 33
    // in the current diff.
    let range = threads[2].anchor.as_ref().unwrap();
    assert_eq!((range.start_line, range.line_new), (Some(28), Some(31)));
    assert_eq!(range.current_line, Some(33));

    // A comment on the removed side of a renamed file.
    let renamed = threads[3].anchor.as_ref().unwrap();
    assert_eq!(renamed.side, Side::Old);
    assert_eq!(renamed.line_old, Some(4));
    assert_eq!(renamed.path_new.as_deref(), Some("src/new_name.rs"));
}

const BASE: &str = "b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0";
const HEAD: &str = "c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3";

#[test]
#[cfg_attr(windows, ignore)]
fn lists_pr_files_across_pages_from_the_merge_base() {
    let fake = script(FAKE);
    let gh = fake.provider("github.com", None);
    let diff = gh
        .fetch_files(
            &widgets_12(),
            &DiffRange::Review {
                base: BASE.into(),
                head: HEAD.into(),
            },
        )
        .unwrap();
    // The left side is the merge base, not the base branch tip.
    assert_eq!(diff.base_sha, "b".repeat(40));
    assert_eq!(diff.head_sha, HEAD);
    assert!(!diff.truncated);
    let paths: Vec<&str> = diff.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "src/cache.rs",
            "src/widget_cache.rs",
            "src/legacy.rs",
            "src/new_name.rs",
            "assets/logo.png",
            "data/fixtures.json"
        ]
    );
    let f = |p: &str| diff.files.iter().find(|f| f.path == p).unwrap();

    let modified = f("src/cache.rs");
    assert_eq!(modified.status, FileStatus::Modified);
    assert_eq!((modified.additions, modified.deletions), (2, 1));
    assert_eq!(modified.content_sha, "f1".repeat(20));
    assert!(modified.patch.as_deref().unwrap().starts_with("@@ -1,3"));
    assert!(!modified.binary && !modified.too_large);

    assert_eq!(f("src/widget_cache.rs").status, FileStatus::Added);
    assert_eq!(f("src/legacy.rs").status, FileStatus::Removed);

    let renamed = f("src/new_name.rs");
    assert_eq!(renamed.status, FileStatus::Renamed);
    assert_eq!(renamed.old_path.as_deref(), Some("src/old_name.rs"));
    assert_eq!(modified.old_path, None);

    // No patch and no line counted: binary. No patch with lines: too large.
    let png = f("assets/logo.png");
    assert!(png.binary && !png.too_large && png.patch.is_none());
    let big = f("data/fixtures.json");
    assert!(big.too_large && !big.binary);
    assert_eq!((big.additions, big.deletions), (4200, 3100));

    assert!(fake
        .calls()
        .contains("api --paginate repos/acme-fixture/widgets/pulls/12/files?per_page=100"));
}

#[test]
#[cfg_attr(windows, ignore)]
fn compares_since_the_last_review() {
    let fake = script(FAKE);
    let gh = fake.provider("github.com", None);
    let from = "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1";
    let diff = gh
        .fetch_files(
            &widgets_12(),
            &DiffRange::Compare {
                from: from.into(),
                to: HEAD.into(),
            },
        )
        .unwrap();
    assert_eq!(diff.base_sha, from);
    assert_eq!(diff.head_sha, HEAD);
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].additions, 1);
    // Never a sha the shell could read as a flag.
    assert!(matches!(
        gh.fetch_files(
            &widgets_12(),
            &DiffRange::Compare {
                from: "--paginate".into(),
                to: HEAD.into()
            }
        ),
        Err(CoreError::InvalidInput(_))
    ));
}

#[test]
#[cfg_attr(windows, ignore)]
fn file_at_decodes_base64_content() {
    let fake = script(FAKE);
    let gh = fake.provider("github.com", None);
    let sha = "c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3";
    let text = gh.file_at(&widgets_12(), sha, "src/cache.rs").unwrap();
    assert_eq!(text, "pub fn ttl() -> u64 {\n    60\n}\n");
    assert!(matches!(
        gh.file_at(&widgets_12(), sha, "big.bin"),
        Err(CoreError::Provider(_))
    ));
    assert!(matches!(
        gh.file_at(&widgets_12(), sha, "missing.rs"),
        Err(CoreError::NotFound)
    ));
    assert!(matches!(
        gh.file_at(&widgets_12(), "--x", "src/cache.rs"),
        Err(CoreError::InvalidInput(_))
    ));
}

#[test]
#[cfg_attr(windows, ignore)]
fn account_token_goes_to_gh_only_via_env() {
    let fake = script(FAKE);
    fake.provider("github.com", Some("work"))
        .current_user()
        .unwrap();
    let calls = fake.calls();
    assert!(calls.contains("api user token=fake-gh-token ent=none host=none"));

    let fake = script(FAKE);
    fake.provider("github.com", None).current_user().unwrap();
    assert!(fake
        .calls()
        .contains("api user token=none ent=none host=none"));

    // GHE: host via GH_HOST, token via GH_ENTERPRISE_TOKEN.
    let fake = script(FAKE);
    fake.provider("ghe.example.test", Some("work"))
        .current_user()
        .unwrap();
    assert!(fake
        .calls()
        .contains("api user token=none ent=fake-ghe-token host=ghe.example.test"));
}

#[test]
#[cfg_attr(windows, ignore)]
fn unavailable_gh_is_an_error_not_a_panic() {
    let missing = GithubProvider::with_binary("/nonexistent/argus-fake-gh", "github.com", None);
    assert!(matches!(
        missing.list_reviews(),
        Err(CoreError::GithubUnavailable)
    ));

    let fake = script(FAKE);
    assert!(matches!(
        fake.provider("github.com", Some("nobody")).current_user(),
        Err(CoreError::GithubUnavailable)
    ));

    let logged_out =
        script("echo 'To get started with GitHub CLI, please run:  gh auth login' >&2; exit 4");
    let err = logged_out
        .provider("github.com", None)
        .list_reviews()
        .unwrap_err();
    assert!(matches!(err, CoreError::GithubUnavailable));
    assert!(err.is_unavailable());

    let noisy = script("echo 'secret-ish stderr' >&2; exit 1");
    let err = noisy
        .provider("github.com", None)
        .list_reviews()
        .unwrap_err();
    assert!(matches!(err, CoreError::Provider(_)));
    assert!(!err.to_string().contains("secret-ish"));

    let limited = script("echo 'API rate limit exceeded for user' >&2; exit 1");
    let err = limited
        .provider("github.com", None)
        .current_user()
        .unwrap_err();
    assert!(err.to_string().contains("rate limit"));
}

#[test]
#[cfg_attr(windows, ignore)]
fn auth_status_checks_gh_locally() {
    use argus_core::provider::GhAuth;

    let fake = script(FAKE);
    assert_eq!(
        fake.provider("github.com", Some("work")).auth_status(),
        GhAuth {
            installed: true,
            logged_in: true
        }
    );
    // No token for this account (and for the active account the fake has
    // none either).
    assert_eq!(
        fake.provider("github.com", Some("nobody")).auth_status(),
        GhAuth {
            installed: true,
            logged_in: false
        }
    );
    assert!(fake
        .calls()
        .contains("auth token --hostname github.com --user work"));
    // Never a network call.
    assert!(!fake.calls().contains("api "));

    let missing = GithubProvider::with_binary("/nonexistent/argus-fixture/gh", "github.com", None);
    assert_eq!(
        missing.auth_status(),
        GhAuth {
            installed: false,
            logged_in: false
        }
    );
}
