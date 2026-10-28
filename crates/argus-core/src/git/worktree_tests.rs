//! Review worktrees over real temp repositories: a "platform" repository,
//! a bare copy of it as the clone's remote (a local path: no network), and
//! the clone.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::*;
use crate::git::tests::{git_in, Fixture};
use crate::git::LocalClone;
use crate::provider::{ProviderId, ReviewRef};
use crate::testutil::review_ref;

/// The platform repository, its bare remote and a clone of that remote.
struct Remote {
    fx: Fixture,
    bare: PathBuf,
    clone: PathBuf,
    /// The commit the clone has (the platform's first one).
    base: String,
}

fn setup() -> Option<Remote> {
    let fx = Fixture::new()?;
    let base = fx.commit("src/a.rs", "fn a() {}\n", "base");
    let root = fx.path.parent().unwrap().to_path_buf();
    let bare = root.join("remote.git");
    git_in(
        &root,
        &[
            "clone",
            "-q",
            "--bare",
            fx.path.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    let clone = root.join("clone");
    git_in(
        &root,
        &[
            "clone",
            "-q",
            bare.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    let clone = clone.canonicalize().unwrap();
    Some(Remote {
        fx,
        bare,
        clone,
        base,
    })
}

impl Remote {
    /// A commit only the remote has, under the platform's ref of `review`.
    fn push_review_head(&self, review: &ReviewRef, text: &str) -> String {
        let head = self.fx.commit("src/a.rs", text, "the change");
        let spec = format!("HEAD:{}", head_ref(review));
        git_in(
            &self.fx.path,
            &["push", "-q", self.bare.to_str().unwrap(), &spec],
        );
        head
    }

    fn open(&self) -> LocalClone {
        LocalClone::open(&self.clone).unwrap()
    }

    fn refs(&self) -> String {
        git_in(
            &self.clone,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        )
    }

    fn worktrees(&self) -> String {
        git_in(&self.clone, &["worktree", "list", "--porcelain"])
    }

    fn root(&self) -> PathBuf {
        self.clone.join(WORKTREES_DIR)
    }
}

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_worktree_is_made_at_the_fetched_head_and_removed_on_drop() {
    let Some(r) = setup() else { return };
    let review = review_ref(12);
    let head = r.push_review_head(&review, "fn a() { changed() }\n");
    let refs_before = r.refs();
    let clone = r.open();
    assert!(
        !clone.has_commit(&head),
        "the clone starts without the head"
    );

    let wt = clone.review_worktree(&review, &head, &no_cancel()).unwrap();
    assert!(wt.fetched());
    assert_eq!(wt.head(), head);
    let path = wt.path().to_path_buf();
    assert_eq!(path.parent().unwrap(), r.root());
    let name = path.file_name().unwrap().to_str().unwrap().to_string();
    assert!(
        name.starts_with(&format!("github-12-{}-", &head[..7])),
        "{name}"
    );
    assert!(valid_slug(&name), "{name}");
    // The head's files, checked out detached; the clone's own checkout and
    // refs are as they were, and no FETCH_HEAD was written.
    assert_eq!(
        std::fs::read_to_string(path.join("src/a.rs")).unwrap(),
        "fn a() { changed() }\n"
    );
    assert_eq!(
        git_in(&path, &["rev-parse", "HEAD"]).trim(),
        head,
        "detached at the head"
    );
    assert_eq!(
        std::fs::read_to_string(r.clone.join("src/a.rs")).unwrap(),
        "fn a() {}\n"
    );
    assert_eq!(git_in(&r.clone, &["rev-parse", "HEAD"]).trim(), r.base);
    assert_eq!(r.refs(), refs_before, "no ref moved");
    assert!(!r.clone.join(".git/FETCH_HEAD").exists());
    // The clone's status stays clean: `.argus/` is excluded.
    assert_eq!(git_in(&r.clone, &["status", "--porcelain"]), "");
    assert!(is_marker(&sidecar(&path)));
    assert!(r.worktrees().contains(path.to_str().unwrap()));

    drop(wt);
    assert!(!path.exists());
    assert!(!sidecar(&path).exists());
    assert!(!r.worktrees().contains(path.to_str().unwrap()));
    assert_eq!(git_in(&r.clone, &["status", "--porcelain"]), "");
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_head_the_clone_has_is_not_fetched() {
    let Some(r) = setup() else { return };
    let review = review_ref(12);
    let clone = r.open();
    // The remote is gone: any fetch would fail. The head is here already.
    std::fs::remove_dir_all(&r.bare).unwrap();
    let wt = clone
        .review_worktree(&review, &r.base, &no_cancel())
        .unwrap();
    assert!(!wt.fetched());
    assert_eq!(
        std::fs::read_to_string(wt.path().join("src/a.rs")).unwrap(),
        "fn a() {}\n"
    );
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_head_the_remote_cannot_send_is_an_error_and_leaves_nothing() {
    let Some(r) = setup() else { return };
    let review = review_ref(12);
    let clone = r.open();
    let missing = "ab".repeat(20);
    let err = clone
        .review_worktree(&review, &missing, &no_cancel())
        .unwrap_err();
    assert!(
        matches!(err, WorktreeError::Fetch(_) | WorktreeError::NotServed),
        "{err:?}"
    );
    assert!(
        err.to_string().contains("head commit is not in the clone"),
        "{err}"
    );
    // Nothing was made: no worktree, no sidecar.
    let left: Vec<_> = std::fs::read_dir(r.root())
        .map(|d| d.filter_map(Result::ok).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
    assert_eq!(r.worktrees().matches("worktree ").count(), 1);
    // A head that is no sha never reaches git.
    assert_eq!(
        clone
            .review_worktree(&review, "--upload-pack=x", &no_cancel())
            .unwrap_err(),
        WorktreeError::BadHead
    );
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_cancelled_run_makes_no_worktree() {
    let Some(r) = setup() else { return };
    let review = review_ref(12);
    let head = r.push_review_head(&review, "x\n");
    let clone = r.open();
    let cancel = AtomicBool::new(true);
    assert_eq!(
        clone.review_worktree(&review, &head, &cancel).unwrap_err(),
        WorktreeError::Cancelled
    );
    assert!(!clone.has_commit(&head));
    assert_eq!(r.worktrees().matches("worktree ").count(), 1);
}

#[test]
#[cfg_attr(windows, ignore)]
fn the_exclude_line_is_added_once() {
    let Some(r) = setup() else { return };
    let review = review_ref(12);
    let clone = r.open();
    let exclude = r.clone.join(".git/info/exclude");
    let before = std::fs::read_to_string(&exclude).unwrap_or_default();
    drop(
        clone
            .review_worktree(&review, &r.base, &no_cancel())
            .unwrap(),
    );
    drop(
        clone
            .review_worktree(&review, &r.base, &no_cancel())
            .unwrap(),
    );
    let after = std::fs::read_to_string(&exclude).unwrap();
    assert!(after.starts_with(&before));
    assert_eq!(after.matches(EXCLUDE_LINE).count(), 1, "{after}");

    // The file on its own: created when missing, a user's own line for
    // `.argus/` counts, an unterminated last line is kept whole.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("info/exclude");
    add_exclude(&file).unwrap();
    add_exclude(&file).unwrap();
    assert_eq!(
        std::fs::read_to_string(&file)
            .unwrap()
            .matches(EXCLUDE_LINE)
            .count(),
        1
    );
    std::fs::write(&file, "*.log\n.argus/\n").unwrap();
    add_exclude(&file).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "*.log\n.argus/\n");
    std::fs::write(&file, "*.log").unwrap();
    add_exclude(&file).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.starts_with("*.log\n") && text.ends_with("/.argus/\n"),
        "{text}"
    );
}

#[test]
#[cfg_attr(windows, ignore)]
fn stale_worktrees_are_swept_by_their_marker_only() {
    let Some(r) = setup() else { return };
    let review = review_ref(12);
    let clone = r.open();
    // A worktree an earlier Argus left behind: forget it without dropping.
    let stale = clone
        .review_worktree(&review, &r.base, &no_cancel())
        .unwrap();
    let stale_path = stale.path().to_path_buf();
    std::mem::forget(stale);
    live().lock().unwrap().remove(&stale_path);
    // A worktree of the user's own in Argus' folder, without a marker; one
    // elsewhere; and a sidecar-looking file that is not Argus'.
    let foreign = r.root().join("github-12-aaaaaaa-bbbbbbbb");
    git_in(
        &r.clone,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            foreign.to_str().unwrap(),
            &r.base,
        ],
    );
    let elsewhere = r.clone.parent().unwrap().join("mine");
    git_in(
        &r.clone,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            elsewhere.to_str().unwrap(),
            &r.base,
        ],
    );
    let fake = r.root().join("github-13-ccccccc-dddddddd.argus");
    std::fs::write(&fake, "not a marker\n").unwrap();
    let fake_dir = r.root().join("github-13-ccccccc-dddddddd");
    std::fs::create_dir(&fake_dir).unwrap();

    assert_eq!(clone.sweep_review_worktrees(), 1);
    assert!(!stale_path.exists());
    assert!(!sidecar(&stale_path).exists());
    let listed = r.worktrees();
    assert!(!listed.contains(stale_path.to_str().unwrap()), "{listed}");
    for kept in [&foreign, &elsewhere] {
        assert!(kept.join("src/a.rs").exists(), "{kept:?}");
        let canonical = kept.canonicalize().unwrap();
        assert!(listed.contains(canonical.to_str().unwrap()), "{listed}");
    }
    assert!(fake.exists() && fake_dir.exists());
    // Nothing left to sweep.
    assert_eq!(clone.sweep_review_worktrees(), 0);
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_half_made_worktree_with_a_marker_is_swept_too() {
    let Some(r) = setup() else { return };
    let clone = r.open();
    // As a killed `git worktree add` leaves it: the marker and a directory
    // git never registered.
    let path = r.root().join("gitlab-3-1234567-89abcdef");
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::write(path.join("src/a.rs"), "half\n").unwrap();
    std::fs::write(sidecar(&path), format!("{MARKER}\n")).unwrap();
    assert_eq!(clone.sweep_review_worktrees(), 1);
    assert!(!path.exists() && !sidecar(&path).exists());
    // Nothing is swept where Argus never made its folder.
    let plain = Fixture::new().unwrap();
    plain.commit("x", "x\n", "x");
    assert_eq!(
        LocalClone::open(&plain.path)
            .unwrap()
            .sweep_review_worktrees(),
        0
    );
    assert!(!plain.path.join(".argus").exists());
}

#[test]
#[cfg_attr(windows, ignore)]
fn two_runs_on_one_clone_each_get_their_own_worktree() {
    let Some(r) = setup() else { return };
    let (a, b) = (review_ref(12), review_ref(13));
    let head_a = r.push_review_head(&a, "a\n");
    let head_b = r.push_review_head(&b, "b\n");
    let clone = std::sync::Arc::new(r.open());
    let run = |review: ReviewRef, head: String| {
        let clone = clone.clone();
        std::thread::spawn(move || clone.review_worktree(&review, &head, &no_cancel()).unwrap())
    };
    let (ta, tb) = (run(a, head_a.clone()), run(b, head_b.clone()));
    let (wa, wb) = (ta.join().unwrap(), tb.join().unwrap());
    assert_ne!(wa.path(), wb.path());
    assert_eq!(
        std::fs::read_to_string(wa.path().join("src/a.rs")).unwrap(),
        "a\n"
    );
    assert_eq!(
        std::fs::read_to_string(wb.path().join("src/a.rs")).unwrap(),
        "b\n"
    );
    // A sweep (another run starting) leaves both alone: they are in use.
    assert_eq!(clone.sweep_review_worktrees(), 0);
    let pa = wa.path().to_path_buf();
    drop(wa);
    assert!(!pa.exists());
    assert!(wb.path().join("src/a.rs").exists());
}

#[test]
fn remote_urls_compare_as_host_and_path() {
    for (url, want) in [
        (
            "https://github.com/acme-fixture/widgets.git",
            Some("github.com/acme-fixture/widgets"),
        ),
        (
            "git@github.com:Acme-Fixture/Widgets.git",
            Some("github.com/acme-fixture/widgets"),
        ),
        (
            "ssh://git@gitlab.example.test:2222/group/sub/proj.git/",
            Some("gitlab.example.test/group/sub/proj"),
        ),
        (
            "https://oauth2:glpat-fixture-xyz@gitlab.example.test:8443/group/proj",
            Some("gitlab.example.test/group/proj"),
        ),
        ("/srv/git/widgets.git", None),
        ("./relative/widgets", None),
    ] {
        assert_eq!(normalize_remote_url(url).as_deref(), want, "{url}");
    }
    assert!(safe_remote_name("upstream") && safe_remote_name("my.fork/x"));
    assert!(!safe_remote_name("--upload-pack=x") && !safe_remote_name("a b"));
    let mut gl = review_ref(3);
    gl.provider = ProviderId::Gitlab;
    assert_eq!(head_ref(&gl), "refs/merge-requests/3/head");
    assert_eq!(head_ref(&review_ref(12)), "refs/pull/12/head");
}

#[test]
#[cfg_attr(windows, ignore)]
fn the_remote_is_the_one_whose_url_is_the_reviews_repository() {
    let Some(r) = setup() else { return };
    let repo = Repo {
        git: r.open().git.clone(),
        dir: r.clone.clone(),
    };
    // `origin` is a local path: no match, so `origin` by default.
    assert_eq!(
        repo.remote_for("github.com", "acme-fixture/widgets"),
        "origin"
    );
    git_in(
        &r.clone,
        &[
            "remote",
            "add",
            "upstream",
            "git@github.com:acme-fixture/widgets.git",
        ],
    );
    assert_eq!(
        repo.remote_for("github.com", "acme-fixture/widgets"),
        "upstream"
    );
    assert_eq!(
        repo.remote_for("github.com", "acme-fixture/other"),
        "origin"
    );
}

#[test]
fn the_fetch_env_keeps_the_users_git_auth_and_no_token() {
    let vars = [
        ("PATH", "/usr/bin"),
        ("HOME", "/Users/fixture"),
        ("SSH_AUTH_SOCK", "/private/tmp/fixture-agent.sock"),
        ("GCM_INTERACTIVE", "never"),
        ("GIT_SSH_COMMAND", "ssh -i ~/.ssh/fixture"),
        ("GIT_ASKPASS", "/usr/local/bin/fixture-askpass"),
        ("GH_TOKEN", "fixture-gh-token"),
        ("GITHUB_TOKEN", "fixture-github-token"),
        ("GITLAB_TOKEN", "glpat-fixture-token"),
        ("CI_JOB_TOKEN", "fixture-ci-token"),
        ("GIT_DIR", "/elsewhere/.git"),
        ("GIT_CONFIG_GLOBAL", "/elsewhere/gitconfig"),
        ("GIT_WORK_TREE", "/elsewhere"),
    ]
    .map(|(k, v)| (OsString::from(k), OsString::from(v)));
    let env = write_env(vars, Some("/opt/homebrew/bin:/usr/bin"));
    let get = |k: &str| {
        env.iter()
            .filter(|(n, _)| n == k)
            .map(|(_, v)| v.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(get("PATH"), ["/opt/homebrew/bin:/usr/bin"]);
    assert_eq!(get("HOME"), ["/Users/fixture"]);
    assert_eq!(get("SSH_AUTH_SOCK"), ["/private/tmp/fixture-agent.sock"]);
    assert_eq!(get("GCM_INTERACTIVE"), ["never"]);
    assert_eq!(get("GIT_SSH_COMMAND"), ["ssh -i ~/.ssh/fixture"]);
    assert_eq!(get("GIT_ASKPASS"), ["/usr/local/bin/fixture-askpass"]);
    assert_eq!(get("GIT_TERMINAL_PROMPT"), ["0"]);
    assert_eq!(get("GIT_LFS_SKIP_SMUDGE"), ["1"]);
    for gone in ["GIT_DIR", "GIT_CONFIG_GLOBAL", "GIT_WORK_TREE"] {
        assert!(get(gone).is_empty(), "{gone}");
    }
    let all: String = env
        .iter()
        .map(|(k, v)| format!("{}={}\n", k.to_string_lossy(), v.to_string_lossy()))
        .collect();
    assert!(!all.contains("token"), "{all}");
    // Without a login shell, Argus' own PATH stays.
    let env = write_env([(OsString::from("PATH"), OsString::from("/bin"))], None);
    assert!(env.iter().any(|(k, v)| k == "PATH" && v == "/bin"));
}

#[test]
#[cfg(unix)]
fn every_changing_git_command_runs_without_hooks_or_tokens() {
    // A fake git that logs its argv and environment, then fails: every
    // changing command Argus runs comes through it.
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let git = dir.path().join("git");
    crate::testutil::write_executable(
        &git,
        &format!(
            "#!/bin/sh\nprintf 'ARGS %s\\n' \"$*\" >> '{}'\nenv >> '{}'\nexit 1\n",
            log.display(),
            log.display()
        ),
    );
    let repo = Repo {
        git: git.clone(),
        dir: dir.path().to_path_buf(),
    };
    let vars = [
        ("HOME", "/Users/fixture"),
        ("SSH_AUTH_SOCK", "/private/tmp/fixture-agent.sock"),
        ("GH_TOKEN", "fixture-gh-token"),
        ("GITHUB_TOKEN", "fixture-github-token"),
        ("GITLAB_TOKEN", "glpat-fixture-token"),
        ("GIT_DIR", "/elsewhere/.git"),
    ]
    .map(|(k, v)| (OsString::from(k), OsString::from(v)));
    let _ = repo.write_in(
        &["fetch", "origin", "refs/pull/1/head"],
        TIMEOUT,
        None,
        vars,
    );
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(
        text.contains("SSH_AUTH_SOCK=/private/tmp/fixture-agent.sock"),
        "{text}"
    );
    assert!(text.contains("HOME=/Users/fixture"), "{text}");
    assert!(
        !text.contains("fixture-gh-token") && !text.contains("glpat-fixture"),
        "{text}"
    );
    assert!(!text.contains("GIT_DIR="), "{text}");
    assert!(text.contains("-c core.hooksPath=/dev/null"), "{text}");
    assert!(text.contains("-c submodule.recurse=false"), "{text}");
    assert!(text.contains("GIT_LFS_SKIP_SMUDGE=1"), "{text}");
    for var in ["GH_TOKEN=", "GITHUB_TOKEN=", "GITLAB_TOKEN="] {
        assert!(!text.contains(var), "{var} in {text}");
    }
}

#[test]
fn slugs_are_recognised_exactly() {
    let review = review_ref(12);
    let s = slug(&review, &"a".repeat(40));
    assert!(valid_slug(&s), "{s}");
    for bad in [
        "github-12-aaaaaaa",
        "bitbucket-12-aaaaaaa-bbbbbbbb",
        "github-x-aaaaaaa-bbbbbbbb",
        "github-12-aaaaaag-bbbbbbbb",
        "github-12-aaaaaaa-bbbbbbbb-c",
        "../github-12-aaaaaaa-bbbbbbbb",
    ] {
        assert!(!valid_slug(bad), "{bad}");
    }
    let p = Path::new("/c/.argus/worktrees/github-12-aaaaaaa-bbbbbbbb");
    assert_eq!(
        sidecar(p),
        Path::new("/c/.argus/worktrees/github-12-aaaaaaa-bbbbbbbb.argus")
    );
}

#[test]
#[cfg(unix)]
fn a_hook_the_branch_ships_never_runs() {
    let Some(r) = setup() else { return };
    let review = review_ref(12);
    // The branch adds a husky-style post-checkout hook, and the clone (as
    // husky sets it up) takes its hooks from the versioned folder.
    let marker = r.clone.parent().unwrap().join("hook-ran");
    let hook = r.fx.path.join(".husky/post-checkout");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    crate::testutil::write_executable(&hook, &format!("#!/bin/sh\ntouch '{}'\n", marker.display()));
    let head = r.push_review_head(&review, "x\n");
    git_in(&r.clone, &["config", "core.hooksPath", ".husky"]);
    let clone = r.open();
    let wt = clone.review_worktree(&review, &head, &no_cancel()).unwrap();
    assert!(wt.path().join(".husky/post-checkout").exists());
    drop(wt);
    assert!(!marker.exists(), "the branch's hook ran");
}
