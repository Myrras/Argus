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
///
/// REST reads come as `api -i [-H 'If-None-Match: <etag>'] <path> [--jq f]`
/// and are answered like `gh api -i` prints them: status line, headers, a
/// blank line, the body. Each fixture's ETag is `"fx-<file name>"`; a
/// request naming it gets a 304 (logged as `304 <path>`), as GitHub answers
/// an unchanged resource. Pages link to the next through `Link`. A
/// `graphql-down` file next to the log makes GraphQL fail.
const FAKE: &str = r##"
echo "$* token=${GH_TOKEN:-none} ent=${GH_ENTERPRISE_TOKEN:-none} host=${GH_HOST:-none}" >> "$LOG"
case "$*" in
  "auth token --hostname github.com --user work") echo fake-gh-token; exit 0 ;;
  "auth token --hostname ghe.example.test --user work") echo fake-ghe-token; exit 0 ;;
  "auth token"*) echo "no account" >&2; exit 1 ;;
  *"--review-requested=@me"*) cat "$F/search_review_requested.json"; exit 0 ;;
  *"--reviewed-by=@me"*) cat "$F/search_reviewed_by.json"; exit 0 ;;
  *"--commenter=@me"*) cat "$F/search_commenter.json"; exit 0 ;;
  "api graphql "*)
    if [ -e "${LOG%/*}/graphql-down" ]; then
      echo '{"errors":[{"type":"FORBIDDEN","message":"Resource not accessible"}]}'
      echo "gh: Resource not accessible" >&2; exit 1
    fi
    case "$*" in
      *"number=12"*"after=Y3Vyc29yOjI="*) cat "$F/graphql_threads_12_page2.json" ;;
      *"number=12"*) cat "$F/graphql_threads_12_page1.json" ;;
      *) echo '{"data":{"repository":{"pullRequest":null}}}' ;;
    esac
    exit 0 ;;
esac
[ "$1 $2" = "api -i" ] || { echo "gh: Not Found (HTTP 404)" >&2; exit 1; }
shift 2
inm=""; path=""
while [ $# -gt 0 ]; do
  case "$1" in
    -H) inm="${2#If-None-Match: }"; shift 2 ;;
    --jq) shift 2 ;;
    *) path="$1"; shift ;;
  esac
done
# respond FILE [NEXT-PAGE-QUERY]
respond() {
  tag="\"fx-${1##*/}\""
  if [ "$inm" = "$tag" ]; then
    printf 'HTTP/2.0 304 Not Modified\nEtag: %s\r\n\r\n' "$tag"
    echo "304 $path" >> "$LOG"
    echo "gh: HTTP 304" >&2; exit 1
  fi
  printf 'HTTP/2.0 200 OK\nContent-Type: application/json; charset=utf-8\r\nEtag: %s\r\n' "$tag"
  if [ -n "$2" ]; then
    printf 'Link: <https://api.github.com/repositories/1/x?%s>; rel="next"\r\n' "$2"
  fi
  printf '\r\n'
  cat "$1"
  exit 0
}
P=repos/acme-fixture/widgets/pulls/12
C1=c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1
C2=c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2
C3=c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3
B0=b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0
case "$path" in
  user) respond "$F/user.json" ;;
  "$P") respond "$F/pull_12.json" ;;
  "$P/commits?per_page=100") respond "$F/pull_12_commits_page1.json" "per_page=100&page=2" ;;
  "$P/commits?per_page=100&page=2") respond "$F/pull_12_commits_page2.json" ;;
  "$P/reviews?per_page=100") respond "$F/pull_12_reviews.json" ;;
  "$P/comments?per_page=100") respond "$F/pull_12_comments_page1.json" "per_page=100&page=2" ;;
  "$P/comments?per_page=100&page=2") respond "$F/pull_12_comments_page2.json" ;;
  "$P/files?per_page=100") respond "$F/pull_12_files_page1.json" "per_page=100&page=2" ;;
  "$P/files?per_page=100&page=2") respond "$F/pull_12_files_page2.json" ;;
  "repos/acme-fixture/widgets/compare/$B0...$C3") respond "$F/compare_b0_c3_merge_base.txt" ;;
  "repos/acme-fixture/widgets/compare/$C1...$C3") respond "$F/compare_c1_c3.json" ;;
  repos/acme-fixture/gadgets/pulls/7) respond "$F/pull_7.json" ;;
  "repos/acme-fixture/gadgets/pulls/7/commits?per_page=100") respond "$F/empty.json" ;;
  "repos/acme-fixture/gadgets/pulls/7/reviews?per_page=100") respond "$F/pull_7_reviews.json" ;;
  "repos/acme-fixture/widgets/contents/src/cache.rs?ref=$C3") respond "$F/contents_cache.json" ;;
  "repos/acme-fixture/widgets/contents/big.bin?ref=$C3") respond "$F/contents_big.json" ;;
  "repos/acme-fixture/widgets/commits/$C2?per_page=100") respond "$F/commit_c2_page1.json" "per_page=100&page=2" ;;
  "repos/acme-fixture/widgets/commits/$C2?per_page=100&page=2") respond "$F/commit_c2_page2.json" ;;
esac
printf 'HTTP/2.0 404 Not Found\nContent-Type: application/json; charset=utf-8\r\n\r\n{"message":"Not Found"}'
echo "gh: Not Found (HTTP 404)" >&2; exit 1
"##;

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

    let calls = fake.calls();
    assert!(calls.contains("api -i repos/acme-fixture/widgets/pulls/12/files?per_page=100 "));
    assert!(calls.contains("api -i repos/acme-fixture/widgets/pulls/12/files?per_page=100&page=2 "));
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
    assert!(calls.contains("api -i user token=fake-gh-token ent=none host=none"));

    // No account and no token from `gh auth token`: gh authenticates on its
    // own.
    let fake = script(FAKE);
    fake.provider("github.com", None).current_user().unwrap();
    assert!(fake
        .calls()
        .contains("api -i user token=none ent=none host=none"));

    // No account: the active account's token, so Argus knows (and can
    // redact) the token gh sends.
    let fake = script(&FAKE.replacen(
        "case \"$*\" in\n",
        "case \"$*\" in\n  \"auth token --hostname github.com\") echo fake-active-token; exit 0 ;;\n",
        1,
    ));
    fake.provider("github.com", None).current_user().unwrap();
    let calls = fake.calls();
    assert!(calls.contains("auth token --hostname github.com token=none"));
    assert!(calls.contains("api -i user token=fake-active-token ent=none host=none"));

    // GHE: host via GH_HOST, token via GH_ENTERPRISE_TOKEN.
    let fake = script(FAKE);
    fake.provider("ghe.example.test", Some("work"))
        .current_user()
        .unwrap();
    assert!(fake
        .calls()
        .contains("api -i user token=none ent=fake-ghe-token host=ghe.example.test"));
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

#[test]
#[cfg_attr(windows, ignore)]
fn commit_diff_reads_every_page_and_the_first_parent() {
    let fake = script(FAKE);
    let gh = fake.provider("github.com", None);
    let sha = "c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2";
    let diff = gh.commit_diff(&widgets_12(), sha).unwrap();
    assert_eq!(diff.base_sha, "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1");
    assert_eq!(diff.head_sha, sha);
    assert!(!diff.truncated);
    let files: Vec<(&str, FileStatus)> = diff
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.status))
        .collect();
    // The second page's file is there too.
    assert_eq!(
        files,
        [
            ("src/cache.rs", FileStatus::Modified),
            ("src/new_name.rs", FileStatus::Renamed),
            ("src/legacy.rs", FileStatus::Removed),
        ]
    );
    assert!(diff.files[0]
        .patch
        .as_deref()
        .unwrap()
        .starts_with("@@ -0,0"));
    // A pure rename: no patch, and neither binary nor too large.
    let rename = &diff.files[1];
    assert_eq!(rename.old_path.as_deref(), Some("src/old_name.rs"));
    assert!(rename.patch.is_none() && !rename.binary && !rename.too_large);
    // GitHub's REST API has no resolve events.
    assert!(gh.thread_resolutions(&widgets_12()).unwrap().is_empty());
    assert!(matches!(
        gh.commit_diff(&widgets_12(), "--x"),
        Err(CoreError::InvalidInput(_))
    ));
}

// --- M6: conditional requests, GraphQL thread state, rate limits, timeouts ---

#[test]
#[cfg_attr(windows, ignore)]
fn a_second_poll_sends_the_etags_and_replays_the_304s() {
    let fake = script(FAKE);
    let gh = fake.provider("github.com", None);
    let range = DiffRange::Review {
        base: BASE.into(),
        head: HEAD.into(),
    };
    let poll = || {
        (
            gh.fetch_review(&widgets_12()).unwrap(),
            gh.fetch_threads(&widgets_12()).unwrap(),
            gh.fetch_files(&widgets_12(), &range).unwrap(),
        )
    };
    let first = poll();
    let calls = fake.calls();
    assert!(!calls.contains("If-None-Match"));
    assert!(!calls.contains("304 "));

    let second = poll();
    // Exactly what a full fetch gave.
    assert_eq!(first, second);
    let calls = fake.calls();
    let comments = "repos/acme-fixture/widgets/pulls/12/comments?per_page=100";
    assert!(calls.contains(&format!(
        "api -i -H If-None-Match: \"fx-pull_12_comments_page1.json\" {comments} "
    )));
    // Page 2 was still followed: its link came from the cached page 1.
    assert!(calls.contains(&format!("304 {comments}&page=2\n")));
    assert!(calls.contains("304 repos/acme-fixture/widgets/pulls/12\n"));
    assert!(calls.contains("304 repos/acme-fixture/widgets/pulls/12/files?per_page=100&page=2\n"));
    // The `--jq` merge base is cached too.
    assert!(calls.contains("304 repos/acme-fixture/widgets/compare/"));
}

#[test]
#[cfg_attr(windows, ignore)]
fn graphql_marks_resolved_threads_across_pages() {
    let fake = script(FAKE);
    let gh = fake.provider("github.com", None);
    let threads = gh.fetch_threads(&widgets_12()).unwrap();
    let state: Vec<(&str, bool, bool)> = threads
        .iter()
        .map(|t| (t.id.as_str(), t.resolved, t.outdated))
        .collect();
    // 501 resolved (page 1), 530 resolved (page 2), 520 outdated per
    // GraphQL although REST still had a position; a GraphQL thread with no
    // REST counterpart is ignored.
    assert_eq!(
        state,
        [
            ("501", true, false),
            ("510", false, true),
            ("530", true, false),
            ("520", false, true),
        ]
    );
    assert_eq!(threads[3].anchor.as_ref().unwrap().current_line, None);
    assert_eq!(threads[2].anchor.as_ref().unwrap().current_line, Some(33));
    let calls = fake.calls();
    assert!(calls.contains("-f owner=acme-fixture -f name=widgets -F number=12 token="));
    assert!(calls.contains("-F number=12 -f after=Y3Vyc29yOjI= token="));
    // Still no resolve events: GraphQL has no time for them.
    assert!(gh.thread_resolutions(&widgets_12()).unwrap().is_empty());
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_failing_graphql_query_leaves_threads_unresolved() {
    let fake = script(FAKE);
    std::fs::write(fake.log.with_file_name("graphql-down"), "").unwrap();
    let gh = fake.provider("github.com", None);
    let threads = gh.fetch_threads(&widgets_12()).unwrap();
    let ids: Vec<&str> = threads.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["501", "510", "530", "520"]);
    assert!(threads.iter().all(|t| !t.resolved));
    // REST's own outdated stands.
    assert!(threads[1].outdated && !threads[3].outdated);
}

/// A `gh` whose every API call is rate-limited with `headers`.
fn limited(headers: &str) -> Fake {
    script(&format!(
        r#"
echo "$*" >> "$LOG"
printf 'HTTP/2.0 403 Forbidden\nContent-Type: application/json; charset=utf-8\r\n{headers}\r\n{{"message":"API rate limit exceeded for user ID 1."}}'
echo "gh: API rate limit exceeded for user ID 1. (HTTP 403)" >&2; exit 1
"#
    ))
}

fn local_hhmm(t: chrono::DateTime<chrono::Utc>) -> String {
    t.with_timezone(&chrono::Local).format("%H:%M").to_string()
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_rate_limit_backs_off_until_the_reset() {
    use chrono::{TimeZone as _, Utc};

    let reset = Utc.timestamp_opt(Utc::now().timestamp() + 600, 0).unwrap();
    let fake = limited(&format!(
        "X-Ratelimit-Remaining: 0\r\nX-Ratelimit-Reset: {}\r\n",
        reset.timestamp()
    ));
    let gh = fake.provider("github.com", None);
    let err = gh.current_user().unwrap_err();
    assert!(!err.is_unavailable());
    assert_eq!(
        err.to_string(),
        format!(
            "Could not fetch reviews: GitHub is rate limited until {}; Argus tries again then",
            local_hhmm(reset)
        )
    );
    assert_eq!(gh.rate_limited_until(), Some(reset));

    // Nothing more goes out until then.
    let before = fake.calls().lines().count();
    assert!(gh.fetch_review(&widgets_12()).is_err());
    assert!(gh.list_reviews().is_err());
    assert_eq!(fake.calls().lines().count(), before);

    // Through the core: one provider-level error, nothing stored.
    let core = argus_core::Core::with_providers(
        argus_core::store::Store::in_memory().unwrap(),
        vec![std::sync::Arc::new(gh)],
    );
    let report = core.refresh();
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].review, None);
    assert!(report.errors[0].message.contains("rate limited until"));
    assert_eq!(fake.calls().lines().count(), before);
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_secondary_rate_limit_waits_for_retry_after() {
    let fake = limited("Retry-After: 90\r\n");
    let gh = fake.provider("github.com", None);
    let start = chrono::Utc::now();
    let err = gh.current_user().unwrap_err();
    assert!(err.to_string().contains("GitHub is rate limited until"));
    let until = gh.rate_limited_until().unwrap();
    let wait = (until - start).num_seconds();
    assert!((88..=92).contains(&wait), "{wait}");
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_hung_gh_is_killed_after_the_timeout() {
    let fake = script("exec sleep 10");
    let gh = fake
        .provider("github.com", None)
        .with_timeout(std::time::Duration::from_millis(300));
    let start = std::time::Instant::now();
    let err = gh.fetch_review(&widgets_12()).unwrap_err();
    assert!(err.to_string().contains("gh did not answer in time"));
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}

/// A fake `gh` serving PR 12's review comments from `page1.json` and
/// `page2.json` next to its log, with an ETag per file content and a 304
/// for a request that names it; GraphQL is down.
const PAGED: &str = r##"
echo "$*" >> "$LOG"
case "$*" in "auth token"*) exit 1 ;; "api graphql "*) exit 1 ;; esac
[ "$1 $2" = "api -i" ] || exit 1
shift 2
inm=""; path=""
while [ $# -gt 0 ]; do
  case "$1" in
    -H) inm="${2#If-None-Match: }"; shift 2 ;;
    --jq) shift 2 ;;
    *) path="$1"; shift ;;
  esac
done
D="${LOG%/*}"
case "$path" in
  "repos/acme-fixture/widgets/pulls/12/comments?per_page=100") f="$D/page1.json" ;;
  "repos/acme-fixture/widgets/pulls/12/comments?per_page=100&page=2") f="$D/page2.json" ;;
  *) printf 'HTTP/2.0 404 Not Found\r\n\r\n{"message":"Not Found"}'; exit 1 ;;
esac
tag="\"fx-$(cksum < "$f" | cut -d' ' -f1)\""
if [ "$inm" = "$tag" ]; then
  printf 'HTTP/2.0 304 Not Modified\r\nEtag: %s\r\n\r\n' "$tag"; exit 1
fi
printf 'HTTP/2.0 200 OK\r\nEtag: %s\r\n\r\n' "$tag"
cat "$f"
"##;

/// `n` root review comments with ids from `from`, as JSON.
fn comments(from: u64, n: u64) -> String {
    let items: Vec<String> = (from..from + n)
        .map(|i| {
            format!(
                r#"{{"id":{i},"in_reply_to_id":null,"user":{{"login":"alice-fixture"}},"body":"c{i}","created_at":"2026-10-10T08:00:00Z","path":"src/cache.rs","position":1,"line":1,"side":"RIGHT","commit_id":"c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3","original_commit_id":"c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3"}}"#
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_full_page_replayed_from_the_cache_still_reads_the_next_one() {
    let fake = script(PAGED);
    let dir = fake.log.parent().unwrap().to_path_buf();
    std::fs::write(dir.join("page1.json"), comments(1, 100)).unwrap();
    std::fs::write(dir.join("page2.json"), "[]").unwrap();
    let gh = fake.provider("github.com", None);
    assert_eq!(gh.fetch_threads(&widgets_12()).unwrap().len(), 100);
    // A 101st comment: page 1 is unchanged (a 304 replaying "no next").
    std::fs::write(dir.join("page2.json"), comments(101, 1)).unwrap();
    let threads = gh.fetch_threads(&widgets_12()).unwrap();
    assert_eq!(threads.len(), 101);
    assert_eq!(threads[100].id, "101");
}
