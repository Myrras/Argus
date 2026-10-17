use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::diff::{assemble, parse_raw, split_patch, DiffError};
use super::*;
use crate::history::{LineStatus, ThreadHistory, ThreadResolution};
use crate::provider::{
    CommitInfo, DiffRange, FileStatus, ProviderId, ReviewDetail, ReviewProvider, ReviewRef, Thread,
};
use crate::settings::Settings;
use crate::store::Store;
use crate::testutil::{at, detail, note, thread, AUTHOR, ME};
use crate::Core;

// ------------------------------------------------------------ pure parts

#[test]
fn versions_and_shas() {
    assert_eq!(parse_version("git version 2.52.0\n"), Some((2, 52)));
    assert_eq!(
        parse_version("git version 2.39.5 (Apple Git-154)"),
        Some((2, 39))
    );
    assert_eq!(parse_version("git version 2.45.1.windows.1"), Some((2, 45)));
    assert_eq!(parse_version("hub version 1"), None);
    assert!((2, 39) < NO_LAZY_FETCH_SINCE && (2, 44) >= NO_LAZY_FETCH_SINCE);

    assert!(is_sha("0123abc"));
    assert!(is_sha(&"a".repeat(40)) && is_sha(&"b".repeat(64)));
    for bad in [
        "",
        "abc",
        "c1",
        "--output=x",
        "HEAD",
        "abcdefg",
        &"a".repeat(65),
    ] {
        assert!(!is_sha(bad), "{bad}");
    }
}

#[test]
fn the_environment_has_no_tokens_and_no_git_redirects() {
    let vars = [
        ("PATH", "/usr/bin"),
        ("HOME", "/home/fixture"),
        ("GH_TOKEN", "fixture-gh"),
        ("GITHUB_TOKEN", "fixture-github"),
        ("GITLAB_TOKEN", "fixture-gl"),
        ("GIT_DIR", "/elsewhere/.git"),
        ("git_index_file", "/elsewhere/index"),
        ("GIT_OPTIONAL_LOCKS", "1"),
    ]
    .map(|(k, v)| (OsString::from(k), OsString::from(v)));
    let env = git_env(vars);
    let get = |k: &str| {
        env.iter()
            .filter(|(n, _)| n == k)
            .map(|(_, v)| v.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(get("PATH"), ["/usr/bin"]);
    assert_eq!(get("HOME"), ["/home/fixture"]);
    for gone in [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GITLAB_TOKEN",
        "GIT_DIR",
        "git_index_file",
    ] {
        assert!(get(gone).is_empty(), "{gone}");
    }
    assert_eq!(get("GIT_OPTIONAL_LOCKS"), ["0"]);
    assert_eq!(get("GIT_NO_LAZY_FETCH"), ["1"]);
    assert_eq!(get("GIT_TERMINAL_PROMPT"), ["0"]);
    let all: String = env
        .iter()
        .map(|(k, v)| format!("{}={}", k.to_string_lossy(), v.to_string_lossy()))
        .collect();
    assert!(!all.contains("fixture-g"));
}

const OID_A: &str = "1111111111111111111111111111111111111111";
const OID_B: &str = "2222222222222222222222222222222222222222";
const ZERO: &str = "0000000000000000000000000000000000000000";

fn raw(entries: &[(&str, &str, &[&str])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (status, new_oid, paths) in entries {
        out.extend(format!(":100644 100644 {OID_A} {new_oid} {status}").bytes());
        for p in *paths {
            out.push(0);
            out.extend(p.bytes());
        }
        out.push(0);
    }
    out
}

#[test]
fn raw_entries_and_patch_sections_pair_up() {
    let r = raw(&[
        ("M", OID_B, &["bin"]),
        ("R094", OID_B, &["a b.txt", "c d é.txt"]),
        ("D", ZERO, &["gone.rs"]),
        ("A", OID_B, &["new.rs"]),
    ]);
    let entries = parse_raw(&r).unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[1].status, FileStatus::Renamed);
    assert_eq!(entries[1].old_path.as_deref(), Some("a b.txt"));
    assert_eq!(entries[1].path, "c d é.txt");
    assert_eq!(entries[2].new_oid, "");

    let patch = "diff --git a/bin b/bin\nindex 1..2 100644\nBinary files a/bin and b/bin differ\n\
                 diff --git a/a b.txt b/c d é.txt\nsimilarity index 94%\nrename from a b.txt\n\
                 rename to c d é.txt\n--- a/a b.txt\t\n+++ b/c d é.txt\t\n\
                 @@ -1,2 +1,3 @@\n x\r\n+diff --git a/fake b/fake\n y\n\
                 diff --git a/gone.rs b/gone.rs\ndeleted file mode 100644\n--- a/gone.rs\n\
                 +++ /dev/null\n@@ -1 +0,0 @@\n-z\n\
                 diff --git a/new.rs b/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/new.rs\n\
                 @@ -0,0 +1 @@\n+n\n";
    // An added line that reads like a header is still part of its hunk.
    assert_eq!(split_patch(patch).len(), 4);
    let d = assemble("base", "head", entries, patch).unwrap();
    assert_eq!((d.base_sha.as_str(), d.head_sha.as_str()), ("base", "head"));
    assert!(d.files[0].binary && d.files[0].patch.is_none());
    let renamed = &d.files[1];
    assert_eq!(
        renamed.patch.as_deref(),
        Some("@@ -1,2 +1,3 @@\n x\r\n+diff --git a/fake b/fake\n y")
    );
    assert_eq!((renamed.additions, renamed.deletions), (1, 0));
    assert_eq!(renamed.content_sha, OID_B);
    assert_eq!(d.files[2].status, FileStatus::Removed);
    assert_eq!(d.files[2].deletions, 1);
    assert_eq!(d.files[3].status, FileStatus::Added);
    assert!(!d.truncated);

    // A section too few is refused, not paired up wrongly.
    let short = parse_raw(&r).unwrap();
    assert_eq!(
        assemble("a", "b", short, "diff --git a/bin b/bin\n"),
        Err(DiffError::Malformed)
    );
    // A type change is the API's to answer.
    assert_eq!(
        parse_raw(&raw(&[("T", OID_B, &["link"])])),
        Err(DiffError::TypeChange)
    );
    assert_eq!(parse_raw(b"garbage\0x\0"), Err(DiffError::Malformed));
    assert_eq!(parse_raw(b"").unwrap(), []);
}

#[test]
fn batch_records_parse() {
    let out = b"aaaa commit 3\nabc\nbbbb blob 4\nx\ny\n\nname:path missing\n";
    let mut rest = &out[..];
    assert_eq!(
        batch_record(&mut rest),
        Some((BatchKind::Object("commit"), &b"abc"[..]))
    );
    assert_eq!(
        batch_record(&mut rest),
        Some((BatchKind::Object("blob"), &b"x\ny\n"[..]))
    );
    assert_eq!(batch_record(&mut rest), Some((BatchKind::Missing, &[][..])));
    assert_eq!(batch_record(&mut rest), None);
    let mut short = &b"aaaa blob 10\nabc\n"[..];
    assert_eq!(batch_record(&mut short), None);
    let mut odd = &b"abc ambiguous\n"[..];
    assert_eq!(batch_record(&mut odd), Some((BatchKind::Other, &[][..])));
}

// ------------------------------------------------------- the runner

#[cfg(unix)]
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[test]
#[cfg(unix)]
fn a_wedged_git_is_killed_at_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let slow = script(dir.path(), "git", "sleep 10");
    let started = Instant::now();
    let r = run(&slow, None, &[], None, Duration::from_millis(200));
    assert_eq!(r.unwrap_err(), GitError::Timeout);
    assert!(started.elapsed() < Duration::from_secs(5));
    // Opening a clone through it gives up the same way: the API answers.
    assert!(
        LocalClone::open_with(&slow, (2, 50), dir.path(), Duration::from_millis(200)).is_none()
    );
}

#[test]
#[cfg(unix)]
fn git_runs_without_a_shell_with_the_clean_environment() {
    let dir = tempfile::tempdir().unwrap();
    let echo = script(dir.path(), "git", "printf '%s\\n' \"$@\"; env");
    let out = run(&echo, Some(dir.path()), &["$(id)", "a b"], None, TIMEOUT).unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("-C"));
    assert_eq!(lines.next(), Some(dir.path().to_str().unwrap()));
    // Literal arguments: nothing was expanded or split.
    assert_eq!(lines.next(), Some("$(id)"));
    assert_eq!(lines.next(), Some("a b"));
    assert!(text.lines().any(|l| l == "GIT_OPTIONAL_LOCKS=0"));
    assert!(!text.contains("GH_TOKEN=") && !text.contains("GITHUB_TOKEN="));
    // stdin reaches git, and the exit code comes back.
    let cat = script(dir.path(), "cat-git", "cat; exit 3");
    let out = run(&cat, None, &[], Some(b"in".to_vec()), TIMEOUT).unwrap();
    assert_eq!((out.code, out.stdout.as_slice()), (Some(3), &b"in"[..]));
}

// ------------------------------------------------------- real repositories

/// A fixture repository, through a git isolated from the user's config.
struct Fixture {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn git_in(path: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(path)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture Author")
        .env("GIT_AUTHOR_EMAIL", "author@example.test")
        .env("GIT_COMMITTER_NAME", "Fixture Author")
        .env("GIT_COMMITTER_EMAIL", "author@example.test")
        .env("GIT_AUTHOR_DATE", "2026-10-10T10:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-10-10T10:00:00Z")
        .output()
        .expect("run git");
    assert!(out.status.success(), "git {args:?} failed");
    String::from_utf8(out.stdout).unwrap()
}

impl Fixture {
    fn new() -> Option<Fixture> {
        if Command::new("git").arg("--version").output().is_err() || git_binary().is_none() {
            eprintln!("skipping: git is not installed");
            return None;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("platform");
        std::fs::create_dir(&path).unwrap();
        git_in(&path, &["init", "-q", "-b", "main"]);
        Some(Fixture { _dir: dir, path })
    }

    fn commit(&self, file: &str, text: &str, message: &str) -> String {
        let p = self.path.join(file);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
        git_in(&self.path, &["add", "-A"]);
        git_in(&self.path, &["commit", "-q", "--no-verify", "-m", message]);
        git_in(&self.path, &["rev-parse", "HEAD"])
            .trim()
            .to_string()
    }

    /// A clone of the repository as it is now, next to it.
    fn clone_now(&self) -> PathBuf {
        let to = self.path.parent().unwrap().join("local");
        let (from, dest) = (self.path.to_str().unwrap(), to.to_str().unwrap());
        git_in(self.path.parent().unwrap(), &["clone", "-q", from, dest]);
        to
    }
}

fn file_of(n: u32) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

#[test]
#[cfg_attr(windows, ignore)]
fn only_a_repository_is_a_clone() {
    let Some(fx) = Fixture::new() else {
        return;
    };
    let plain = tempfile::tempdir().unwrap();
    assert!(LocalClone::open(plain.path()).is_none());
    assert!(LocalClone::open(&plain.path().join("missing")).is_none());
    std::fs::write(plain.path().join("file"), "x").unwrap();
    assert!(LocalClone::open(&plain.path().join("file")).is_none());
    let c1 = fx.commit("a.txt", "a\n", "c1");
    let clone = LocalClone::open(&fx.path).unwrap();
    assert!(clone.has_commit(&c1));
    assert!(!clone.has_commit(&"ab".repeat(20)));
    assert!(!clone.has_commit("HEAD"));
    assert_eq!(clone.file_at(&c1, "a.txt").unwrap().unwrap(), "a\n");
    assert!(matches!(
        clone.file_at(&c1, "b.txt"),
        Some(Err(CoreError::NotFound))
    ));
    assert!(clone.file_at(&"ab".repeat(20), "a.txt").is_none());
    // A directory is not a file, and latin-1 bytes are not UTF-8: the API's
    // to answer.
    let c2 = fx.commit("dir/x.txt", "x\n", "c2");
    assert!(clone.file_at(&c2, "dir").is_none());
    let p = fx.path.join("latin1.txt");
    std::fs::write(&p, b"caf\xe9\n").unwrap();
    git_in(&fx.path, &["add", "-A"]);
    git_in(&fx.path, &["commit", "-q", "-m", "c3"]);
    let c3 = git_in(&fx.path, &["rev-parse", "HEAD"]).trim().to_string();
    assert!(clone.file_at(&c3, "latin1.txt").is_none());
}

/// A provider serving a "platform" repository, counting what the history
/// and the code view fetch from it.
struct Platform {
    repo: LocalClone,
    detail: ReviewDetail,
    threads: Vec<Thread>,
    /// What `fetch_files` answers; `None` is a not-found.
    files: std::sync::Mutex<Option<FileDiff>>,
    file_calls: AtomicUsize,
    diff_calls: AtomicUsize,
    /// What `post_comment` was asked to post.
    posted: std::sync::Mutex<Vec<crate::post::PlacedComment>>,
}

impl ReviewProvider for Platform {
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
        Ok(vec![self.detail.review.clone()])
    }
    fn fetch_review(&self, _: &ReviewRef) -> CoreResult<ReviewDetail> {
        Ok(self.detail.clone())
    }
    fn fetch_threads(&self, _: &ReviewRef) -> CoreResult<Vec<Thread>> {
        Ok(self.threads.clone())
    }
    fn file_at(&self, _: &ReviewRef, sha: &str, path: &str) -> CoreResult<String> {
        self.file_calls.fetch_add(1, Ordering::SeqCst);
        self.repo
            .file_at(sha, path)
            .unwrap_or(Err(CoreError::NotFound))
    }
    fn fetch_files(&self, _: &ReviewRef, _: &DiffRange) -> CoreResult<FileDiff> {
        self.files
            .lock()
            .unwrap()
            .clone()
            .ok_or(CoreError::NotFound)
    }
    fn commit_diff(&self, _: &ReviewRef, sha: &str) -> CoreResult<FileDiff> {
        self.diff_calls.fetch_add(1, Ordering::SeqCst);
        self.repo.commit_diff(sha)
    }
    fn thread_resolutions(&self, _: &ReviewRef) -> CoreResult<Vec<ThreadResolution>> {
        Ok(Vec::new())
    }
    fn post_comment(
        &self,
        _: &ReviewRef,
        c: &crate::post::PlacedComment,
    ) -> crate::provider::PostResultOf<crate::PostedComment> {
        self.posted.lock().unwrap().push(c.clone());
        Ok(crate::PostedComment {
            url: "https://github.com/acme-fixture/widgets/pull/12#r1".into(),
            provider_thread_id: "900".into(),
            provider_note_id: "900".into(),
            path: c.path.clone(),
            side: c.side,
            line: c.line_number(),
            start_line: None,
            head_sha: c.head_sha.clone(),
        })
    }
}

/// c1: `src/cache.rs`, 30 lines (the comment is on line 10); c2: two lines
/// in at the top; c3: line 10 rewritten.
fn platform_repo(fx: &Fixture) -> Vec<String> {
    let v1 = file_of(30);
    let v2 = format!("x\ny\n{v1}");
    let v3 = v2.replace("line 10\n", "line ten\n");
    vec![
        fx.commit("src/cache.rs", &v1, "c1"),
        fx.commit("src/cache.rs", &v2, "c2"),
        fx.commit("src/cache.rs", &v3, "c3"),
    ]
}

fn platform(fx: &Fixture, shas: &[String]) -> Arc<Platform> {
    platform_on(fx, shas, "base")
}

/// [`platform`] with `base` as the review's base branch commit.
fn platform_on(fx: &Fixture, shas: &[String], base: &str) -> Arc<Platform> {
    let mut d = detail(shas.last().unwrap());
    d.base_sha = base.into();
    d.commits = shas
        .iter()
        .enumerate()
        .map(|(i, sha)| CommitInfo {
            sha: sha.clone(),
            title: format!("c{}", i + 1),
            author: AUTHOR.into(),
            authored_at: Some(at(10 + i as u32, 9)),
        })
        .collect();
    let mine = thread("100", ME, &shas[0], vec![note("n1", ME, at(10, 11))]);
    Arc::new(Platform {
        repo: LocalClone::open(&fx.path).unwrap(),
        detail: d,
        threads: vec![mine],
        files: std::sync::Mutex::new(None),
        file_calls: AtomicUsize::new(0),
        diff_calls: AtomicUsize::new(0),
        posted: std::sync::Mutex::new(Vec::new()),
    })
}

/// A core over `platform` with `clone` as the repository's local clone.
fn core_with(platform: &Arc<Platform>, clone: &Path) -> Core {
    let mut settings = Settings::default();
    settings.clone_paths.insert(
        "github.com/acme-fixture/widgets".into(),
        clone.to_path_buf(),
    );
    let core = Core::with_parts(
        Store::in_memory().unwrap(),
        None,
        settings,
        Some(vec![platform.clone() as Arc<dyn ReviewProvider>]),
    );
    let report = core.refresh();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    core
}

/// What the user sees of a history, without the per-core ids.
fn seen(h: &ThreadHistory) -> impl PartialEq + std::fmt::Debug + '_ {
    (
        &h.status,
        &h.touched_by,
        &h.then_snapshot,
        &h.now_snapshot,
        &h.diff_lines,
        &h.timeline,
    )
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_commit_missing_from_the_clone_falls_back_to_the_api() {
    let Some(fx) = Fixture::new() else {
        return;
    };
    let shas = platform_repo(&fx);
    // The clone was made at c3 and has everything...
    let full = fx.clone_now();
    // ...until the platform gets c4 (README only), which it has not fetched.
    let mut shas = shas;
    shas.push(fx.commit("README.md", "readme\n", "c4"));
    let (c1, c4) = (&shas[0], &shas[3]);

    let api = platform(&fx, &shas);
    let core = core_with(&api, &full);
    let id = core.reviews().unwrap()[0].id;
    let through_api = core.review_histories(id).unwrap().remove(0);
    assert_eq!(
        through_api.status,
        LineStatus::Located {
            path: "src/cache.rs".into(),
            start_line: 12,
            line: 12,
            moved: true,
            renamed: false,
            touched: true
        }
    );
    assert_eq!(through_api.touched_by, [shas[2].clone()]);
    // The walk went through the API (c4 is not local); the then file at c1
    // still came from the clone, the now file at c4 from the API.
    assert_eq!(api.diff_calls.load(Ordering::SeqCst), 3);
    assert_eq!(api.file_calls.load(Ordering::SeqCst), 1);
    // The code view: the same split.
    assert!(core
        .file_at(id, c1, "src/cache.rs")
        .unwrap()
        .contains("line 10\n"));
    assert_eq!(api.file_calls.load(Ordering::SeqCst), 1);
    assert!(core.file_at(id, c4, "README.md").is_ok());
    assert_eq!(api.file_calls.load(Ordering::SeqCst), 2);

    // Once fetched (by the user, never by Argus), the clone answers it all
    // with the same result and no API call.
    git_in(&full, &["fetch", "-q", "origin"]);
    let api2 = platform(&fx, &shas);
    let core2 = core_with(&api2, &full);
    let id2 = core2.reviews().unwrap()[0].id;
    let through_clone = core2.review_histories(id2).unwrap().remove(0);
    assert_eq!(seen(&through_clone), seen(&through_api));
    assert_eq!(api2.diff_calls.load(Ordering::SeqCst), 0);
    assert_eq!(api2.file_calls.load(Ordering::SeqCst), 0);
    assert_eq!(core2.file_at(id2, c4, "README.md").unwrap(), "readme\n");
    // A file the commit does not have is answered locally too.
    assert!(matches!(
        core2.file_at(id2, c4, "nope.rs"),
        Err(CoreError::NotFound)
    ));
    assert_eq!(api2.file_calls.load(Ordering::SeqCst), 0);
    // A sha the clone does not know goes to the API.
    let _ = core2.file_at(id2, &"ab".repeat(20), "README.md");
    assert_eq!(api2.file_calls.load(Ordering::SeqCst), 1);
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_path_that_is_no_clone_uses_the_api() {
    let Some(fx) = Fixture::new() else {
        return;
    };
    let shas = platform_repo(&fx);
    let reference = {
        let api = platform(&fx, &shas);
        let core = core_with(&api, &fx.clone_now());
        let id = core.reviews().unwrap()[0].id;
        let h = core.review_histories(id).unwrap().remove(0);
        assert_eq!(api.diff_calls.load(Ordering::SeqCst), 0);
        assert_eq!(api.file_calls.load(Ordering::SeqCst), 0);
        h
    };
    let not_a_repo = tempfile::tempdir().unwrap();
    for path in [
        not_a_repo.path().to_path_buf(),
        not_a_repo.path().join("gone"),
    ] {
        let api = platform(&fx, &shas);
        let core = core_with(&api, &path);
        let id = core.reviews().unwrap()[0].id;
        let h = core.review_histories(id).unwrap().remove(0);
        assert_eq!(seen(&h), seen(&reference));
        assert_eq!(api.diff_calls.load(Ordering::SeqCst), 2);
        assert_eq!(api.file_calls.load(Ordering::SeqCst), 2);
    }
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_partial_clone_never_fetches_and_never_guesses() {
    let Some(fx) = Fixture::new() else {
        return;
    };
    if git_binary().is_none_or(|g| g.version < NO_LAZY_FETCH_SINCE) {
        return;
    }
    let shas = platform_repo(&fx);
    git_in(&fx.path, &["config", "uploadpack.allowFilter", "true"]);
    let local = fx.path.parent().unwrap().join("partial");
    let url = format!("file://{}", fx.path.display());
    git_in(
        fx.path.parent().unwrap(),
        &[
            "clone",
            "-q",
            "--filter=blob:none",
            &url,
            local.to_str().unwrap(),
        ],
    );
    let objects = || git_in(&local, &["count-objects", "-v"]);
    let before = objects();

    let clone = LocalClone::open(&local).unwrap();
    assert!(clone.partial);
    // c1's blob is not local: not "no such file", but the API's to answer.
    assert!(clone.has_commit(&shas[0]));
    assert!(clone.file_at(&shas[0], "src/cache.rs").is_none());
    assert!(clone.file_at(&shas[0], "nope.rs").is_none());
    // The checked-out head's blob is there.
    assert!(clone
        .file_at(&shas[2], "src/cache.rs")
        .unwrap()
        .unwrap()
        .contains("line ten"));
    // A diff that needs a missing blob fails (and the walk takes the API).
    assert!(clone.commit_diff(&shas[1]).is_err());
    assert_eq!(objects(), before, "nothing was fetched");
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_clone_made_partial_since_it_was_checked_is_checked_again() {
    let Some(fx) = Fixture::new() else {
        return;
    };
    fx.commit("a.txt", "a\n", "c1");
    let git = git_binary().unwrap().path.clone();
    // An old git (before GIT_NO_LAZY_FETCH): a full clone is fine...
    let old = (2, 40);
    assert!(LocalClone::open_with(&git, old, &fx.path, TIMEOUT).is_some());
    // ...then the directory becomes a partial clone (re-cloned with a
    // filter, here its config marked so): such a git could fetch.
    git_in(&fx.path, &["config", "extensions.partialClone", "origin"]);
    let config = std::fs::File::options()
        .write(true)
        .open(fx.path.join(".git/config"))
        .unwrap();
    // A different mtime even on a filesystem with coarse timestamps.
    config
        .set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000))
        .unwrap();
    assert!(LocalClone::open_with(&git, old, &fx.path, TIMEOUT).is_none());
}

#[test]
#[cfg_attr(windows, ignore)]
fn the_merge_base_of_two_commits() {
    let Some(fx) = Fixture::new() else {
        return;
    };
    let base = fx.commit("a.txt", "a\n", "c1");
    git_in(&fx.path, &["checkout", "-q", "-b", "feature"]);
    let head = fx.commit("b.txt", "b\n", "feature");
    git_in(&fx.path, &["checkout", "-q", "main"]);
    let tip = fx.commit("a.txt", "a2\n", "main moves on");
    let clone = LocalClone::open(&fx.path).unwrap();
    assert_eq!(
        clone.merge_base(&tip, &head).as_deref(),
        Some(base.as_str())
    );
    // A commit the clone lacks, or no sha at all: no answer.
    assert_eq!(clone.merge_base(&"ab".repeat(20), &head), None);
    assert_eq!(clone.merge_base("main", &head), None);
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_pr_without_a_merge_base_gets_it_from_the_clone() {
    // GitHub could not compare a large PR, so its files came with the base
    // branch tip as a stand-in left side. The clone knows the real one.
    let Some(fx) = Fixture::new() else {
        return;
    };
    let base = fx.commit("src/cache.rs", &file_of(10), "c1");
    git_in(&fx.path, &["checkout", "-q", "-b", "feature"]);
    let head = fx.commit("src/cache.rs", &file_of(12), "feature");
    git_in(&fx.path, &["checkout", "-q", "main"]);
    let tip = fx.commit("src/other.rs", "x\n", "main moves on");
    let p = platform(&fx, std::slice::from_ref(&head));
    let approximate = FileDiff {
        base_sha: tip.clone(),
        head_sha: head.clone(),
        files: Vec::new(),
        truncated: false,
        base_approximate: true,
    };
    *p.files.lock().unwrap() = Some(approximate);

    let core = core_with(&p, &fx.path);
    let id = core.reviews().unwrap()[0].id;
    let files = core.list_files(id, false).unwrap();
    assert_eq!(files.base_sha, base);
    assert!(!files.base_approximate);

    // No usable clone: the stand-in stays, and says so.
    let elsewhere = tempfile::tempdir().unwrap();
    let core = core_with(&p, elsewhere.path());
    let files = core.list_files(id, false).unwrap();
    assert_eq!(files.base_sha, tip);
    assert!(files.base_approximate);
}

#[test]
#[cfg_attr(windows, ignore)]
fn an_exact_left_side_is_never_replaced_by_the_clone() {
    // The platform computed the left side itself: the clone's view (here
    // another merge base than the one served) never overrides it.
    let Some(fx) = Fixture::new() else {
        return;
    };
    fx.commit("src/cache.rs", &file_of(10), "c1");
    git_in(&fx.path, &["checkout", "-q", "-b", "feature"]);
    let head = fx.commit("src/cache.rs", &file_of(12), "feature");
    git_in(&fx.path, &["checkout", "-q", "main"]);
    let tip = fx.commit("src/other.rs", "x\n", "main moves on");
    let p = platform_on(&fx, std::slice::from_ref(&head), &tip);
    *p.files.lock().unwrap() = Some(FileDiff {
        base_sha: tip.clone(),
        head_sha: head.clone(),
        files: Vec::new(),
        truncated: false,
        base_approximate: false,
    });
    let core = core_with(&p, &fx.path);
    let id = core.reviews().unwrap()[0].id;
    let files = core.list_files(id, false).unwrap();
    assert_eq!(files.base_sha, tip);
    assert!(!files.base_approximate);
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_removed_line_posts_against_the_left_side_the_clone_found() {
    // The code view showed the clone's merge base as the left side; posting
    // and the commentable lines must read the diff the same way, or every
    // comment on a removed line is refused as read against another side.
    let Some(fx) = Fixture::new() else {
        return;
    };
    let base = fx.commit("src/cache.rs", &file_of(10), "c1");
    git_in(&fx.path, &["checkout", "-q", "-b", "feature"]);
    let head = fx.commit(
        "src/cache.rs",
        &file_of(10).replace("line 10\n", "line ten\n"),
        "feature",
    );
    git_in(&fx.path, &["checkout", "-q", "main"]);
    let tip = fx.commit("src/other.rs", "x\n", "main moves on");
    let p = platform(&fx, std::slice::from_ref(&head));
    let file = crate::provider::ChangedFile {
        path: "src/cache.rs".into(),
        old_path: None,
        status: crate::provider::FileStatus::Modified,
        additions: 1,
        deletions: 1,
        patch: Some("@@ -9,2 +9,2 @@\n line 9\n-line 10\n+line ten\n".into()),
        content_sha: "c1".into(),
        binary: false,
        too_large: false,
    };
    *p.files.lock().unwrap() = Some(FileDiff {
        base_sha: tip,
        head_sha: head.clone(),
        files: vec![file],
        truncated: false,
        base_approximate: true,
    });
    let core = core_with(&p, &fx.path);
    let id = core.reviews().unwrap()[0].id;
    let files = core.list_files(id, false).unwrap();
    assert_eq!(files.base_sha, base);
    let lines = core.commentable_lines(id, "src/cache.rs").unwrap();
    assert_eq!(lines.base_sha, base);

    let results = core
        .post_comments(
            id,
            vec![crate::NewComment {
                path: "src/cache.rs".into(),
                line: 10,
                side: crate::provider::Side::Old,
                start_line: None,
                body: "Why drop the ten?".into(),
                head_sha: Some(head),
                base_sha: Some(files.base_sha),
            }],
        )
        .unwrap();
    assert!(
        matches!(results[0], crate::PostResult::Posted(_)),
        "{results:?}"
    );
    assert_eq!(p.posted.lock().unwrap()[0].base_sha, base);

    // An AI run numbers its old-side findings at the same left side, so
    // they post too.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin = tempfile::tempdir().unwrap();
        let claude = bin.path().join("claude");
        let answer = serde_json::json!({
            "type": "result", "subtype": "success", "is_error": false,
            "result": r#"{"findings":[],"checklist":[],"verdicts":[]}"#
        });
        std::fs::write(
            &claude,
            format!("#!/bin/sh\ncat > /dev/null\necho '{answer}'\n"),
        )
        .unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut settings = core.settings();
        settings.ai.adapter = "claude".into();
        settings.ai.binary = Some(claude.to_string_lossy().into_owned());
        let ctx = crate::ai::job::JobContext {
            settings,
            provider: p.clone(),
            timeout: Duration::from_secs(20),
            child: Default::default(),
        };
        let record = core.review(id).unwrap();
        let out = crate::ai::job::execute(
            &core.store,
            &record,
            &ctx,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(out.base_sha.as_deref(), Some(base.as_str()));
    }
}
