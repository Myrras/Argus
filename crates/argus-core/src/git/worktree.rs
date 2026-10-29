//! Review worktrees: the one place Argus changes a local clone.
//!
//! Before an AI run, the review's head commit is fetched when the clone
//! lacks it (that one ref, no tags, no `FETCH_HEAD`, no remote-tracking ref
//! moved) and checked out, detached, in `<clone>/.argus/worktrees/<slug>`,
//! which the agent reads instead of the clone's own checkout; after the run
//! (done, failed, cancelled, or a panic unwinding through the guard) that
//! worktree is removed again. `.argus/` goes into the clone's
//! `info/exclude` once, so the clone's `git status` stays clean. Nothing
//! else changes: no branch is checked out in the clone, no ref moves,
//! nothing is pushed.
//!
//! Every worktree is marked as Argus' own by a sidecar file
//! `<slug>.argus` next to it, written before git makes the worktree. The
//! sweeps (at startup, and before each run) remove marked worktrees only,
//! and never one a run of this process still uses. Any other worktree,
//! branch or file is left alone.
//!
//! The changing commands run with hooks off (a branch can ship a
//! `post-checkout` hook through a versioned `core.hooksPath`, as husky
//! does), without submodules (a branch's `.gitmodules` names the URLs), Git
//! LFS's download, fsmonitor, automatic gc or maintenance. The fetch uses
//! the user's own git auth (their SSH agent, credential helper and remote
//! config) and never Argus' provider tokens; see [`write_env`].

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use super::{git_env, is_sha, run_with, GitError, LocalClone, Output, TIMEOUT};
use crate::provider::{ProviderId, ReviewRef};

/// Where the worktrees live, relative to the clone's root.
pub(crate) const WORKTREES_DIR: &str = ".argus/worktrees";

/// The first line of a sidecar: what marks a worktree as Argus' own.
const MARKER: &str = "argus-review-worktree v1";

/// The sidecar's extension (`<slug>.argus`).
const SIDECAR_EXT: &str = "argus";

/// The line `info/exclude` gets.
const EXCLUDE_LINE: &str = "/.argus/";

/// A fetch of one ref over the network.
const FETCH_TIMEOUT: Duration = Duration::from_secs(120);

/// A checkout of the review's head (a large repository takes a while).
const ADD_TIMEOUT: Duration = Duration::from_secs(120);

/// Removing a worktree (deleting its files).
const REMOVE_TIMEOUT: Duration = Duration::from_secs(60);

/// Config every changing command runs with (see the module docs).
const WRITE_CONFIG: &[&str] = &[
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "submodule.recurse=false",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "gc.auto=0",
    "-c",
    "maintenance.auto=false",
    "-c",
    "fetch.writeCommitGraph=false",
];

/// `GIT_*` variables of Argus' environment the fetch keeps: they choose how
/// git authenticates (an SSH command, an askpass helper), never where it
/// reads or writes. Every other `GIT_*` one is dropped as in [`git_env`].
const AUTH_GIT_VARS: &[&str] = &[
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_SSH_VARIANT",
    "GIT_ASKPASS",
];

/// The environment of the changing commands: [`git_env`] (no provider
/// token, no `GIT_*` redirection, no prompt), plus the `GIT_*` variables
/// that choose the user's own git auth ([`AUTH_GIT_VARS`]), the login
/// shell's `PATH` (a credential helper such as `gh` or Git Credential
/// Manager lives there, not on a Finder-launched app's `PATH`) and Git
/// LFS's download off. `SSH_AUTH_SOCK`, `HOME` (`~/.gitconfig`, the
/// credential helper's config, `~/.ssh`) and the rest of the user's
/// environment pass as they are.
pub(crate) fn write_env(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    login_path: Option<&str>,
) -> Vec<(OsString, OsString)> {
    let vars: Vec<(OsString, OsString)> = vars.into_iter().collect();
    let auth = |k: &OsStr| {
        let k = k.to_string_lossy().to_ascii_uppercase();
        AUTH_GIT_VARS.contains(&k.as_str())
    };
    let kept: Vec<(OsString, OsString)> = vars.iter().filter(|(k, _)| auth(k)).cloned().collect();
    let mut env: Vec<(OsString, OsString)> = git_env(vars)
        .into_iter()
        .filter(|(k, _)| login_path.is_none() || !k.eq_ignore_ascii_case("PATH"))
        .collect();
    if let Some(p) = login_path {
        env.push(("PATH".into(), p.into()));
    }
    env.extend(kept);
    env.push(("GIT_LFS_SKIP_SMUDGE".into(), "1".into()));
    env
}

/// Worktrees runs of this process use right now: the sweeps never touch
/// them (two runs of reviews of the same repository share its clone).
fn live() -> &'static Mutex<HashSet<PathBuf>> {
    static LIVE: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// One lock per clone: its fetch, worktree add, sweep and removals run one
/// at a time (the agents of two runs still run side by side).
fn clone_lock(dir: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("clone locks poisoned")
        .entry(dir.to_path_buf())
        .or_default()
        .clone()
}

/// Why no review worktree was made. Argus' own text: git's stderr is never
/// read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeError {
    /// The run was cancelled meanwhile.
    Cancelled,
    /// The head is no commit id.
    BadHead,
    /// Fetching the head failed.
    Fetch(GitError),
    /// The fetch ran, but the remote did not have the head commit.
    NotServed,
    /// A file or directory of Argus' own could not be written.
    Setup(&'static str),
    /// `git worktree add` failed.
    Add(GitError),
}

impl std::fmt::Display for WorktreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorktreeError::Cancelled => write!(f, "cancelled"),
            WorktreeError::BadHead => write!(f, "the review's head is not a commit id"),
            WorktreeError::Fetch(e) => write!(
                f,
                "the head commit is not in the clone and fetching it failed ({e}; check that \
                 `git fetch` works in the clone)"
            ),
            WorktreeError::NotServed => write!(
                f,
                "the head commit is not in the clone and the remote did not send it"
            ),
            WorktreeError::Setup(what) => write!(f, "could not write {what}"),
            WorktreeError::Add(e) => write!(f, "git worktree add failed ({e})"),
        }
    }
}

/// A review worktree in use: removed (with its sidecar) when dropped.
pub struct ReviewWorktree {
    repo: Repo,
    path: PathBuf,
    head: String,
    fetched: bool,
}

impl std::fmt::Debug for ReviewWorktree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReviewWorktree")
            .field("path", &self.path)
            .finish()
    }
}

impl ReviewWorktree {
    /// The worktree's root (canonical, under `<clone>/.argus/worktrees`).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The commit it has checked out.
    pub fn head(&self) -> &str {
        &self.head
    }

    /// Whether the head had to be fetched first.
    pub fn fetched(&self) -> bool {
        self.fetched
    }
}

impl Drop for ReviewWorktree {
    fn drop(&mut self) {
        let lock = clone_lock(&self.repo.dir);
        let _one_at_a_time = lock.lock().unwrap_or_else(|e| e.into_inner());
        self.repo.remove(&self.path, false);
        live()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.path);
    }
}

/// The clone a worktree belongs to: its git and root.
#[derive(Debug, Clone)]
struct Repo {
    git: PathBuf,
    dir: PathBuf,
}

impl Repo {
    /// A changing `git` command: [`WRITE_CONFIG`] first, [`write_env`] of
    /// Argus' own environment.
    fn write(
        &self,
        args: &[&str],
        timeout: Duration,
        cancel: Option<&AtomicBool>,
    ) -> Result<Output, GitError> {
        self.write_in(args, timeout, cancel, std::env::vars_os())
    }

    /// [`write`](Self::write) with [`write_env`] of `vars`.
    fn write_in(
        &self,
        args: &[&str],
        timeout: Duration,
        cancel: Option<&AtomicBool>,
        vars: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Output, GitError> {
        let mut all: Vec<&str> = WRITE_CONFIG.to_vec();
        all.extend_from_slice(args);
        let env = write_env(vars, crate::platform::login_path());
        run_with(
            &self.git,
            Some(&self.dir),
            &all,
            None,
            timeout,
            &env,
            cancel,
        )
    }

    fn write_ok(
        &self,
        args: &[&str],
        timeout: Duration,
        cancel: Option<&AtomicBool>,
    ) -> Result<(), GitError> {
        match self.write(args, timeout, cancel)?.code {
            Some(0) => Ok(()),
            code => Err(GitError::Exit(code)),
        }
    }

    /// A read (`git_env`, no write config).
    fn read(&self, args: &[&str]) -> Option<Vec<u8>> {
        super::run(&self.git, Some(&self.dir), args, None, TIMEOUT)
            .ok()
            .filter(|o| o.code == Some(0))
            .map(|o| o.stdout)
    }

    fn root(&self) -> PathBuf {
        self.dir.join(WORKTREES_DIR)
    }

    /// Remove the worktree at `path` (one of ours) and its registration:
    /// `git worktree remove --force` (twice forced for one a killed `add`
    /// left locked), then the directory itself if git left it (half made,
    /// or never registered), then the sidecar. A `git worktree prune`
    /// follows, unless it would also prune a worktree that is not Argus'
    /// (one whose directory is missing just now, on a volume that is not
    /// mounted, say): the remove has already dropped ours.
    fn remove(&self, path: &Path, locked_too: bool) {
        let p = path.to_string_lossy();
        let mut args = vec!["worktree", "remove", "--force"];
        if locked_too {
            args.push("--force");
        }
        args.push(&p);
        if let Err(e) = self.write_ok(&args, REMOVE_TIMEOUT, None) {
            tracing::debug!("review worktree: git worktree remove failed ({e})");
        }
        let ours = path.parent() == Some(self.root().as_path());
        let is_dir = path
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_dir());
        if ours && is_dir {
            if let Err(e) = std::fs::remove_dir_all(path) {
                tracing::debug!("review worktree: could not delete a leftover directory: {e}");
            }
        }
        let _ = std::fs::remove_file(sidecar(path));
        if self.only_ours_prunable() {
            if let Err(e) = self.write_ok(&["worktree", "prune"], TIMEOUT, None) {
                tracing::debug!("review worktree: git worktree prune failed ({e})");
            }
        }
    }

    /// Whether every worktree `git worktree prune` would prune now (its
    /// directory is gone) is one of Argus' own.
    fn only_ours_prunable(&self) -> bool {
        let Some(out) = self.read(&["worktree", "list", "--porcelain", "-z"]) else {
            return false;
        };
        let root = self.root();
        out.split(|b| *b == 0)
            .filter_map(|line| std::str::from_utf8(line).ok())
            .filter_map(|line| line.strip_prefix("worktree "))
            .map(PathBuf::from)
            .filter(|p| !p.exists())
            .all(|p| p.parent() == Some(root.as_path()))
    }

    /// Make sure `head` is a commit of the clone: fetched from the review's
    /// remote when missing. Whether it had to be fetched.
    fn ensure_head(
        &self,
        clone: &LocalClone,
        review: &ReviewRef,
        head: &str,
        cancel: &AtomicBool,
    ) -> Result<bool, WorktreeError> {
        if clone.has_commit(head) {
            return Ok(false);
        }
        let remote = self.remote_for(&review.host, &review.repo);
        // The platform's own ref first (every host serves it), then the
        // commit itself, for a head the ref has moved past since the review
        // was read (served when the host allows fetching by id).
        let mut failed = None;
        for spec in [head_ref(review), head.to_string()] {
            let fetched = self.write_ok(
                &[
                    "fetch",
                    "--quiet",
                    "--no-tags",
                    "--no-write-fetch-head",
                    "--no-recurse-submodules",
                    // No remote-tracking ref is updated on the side, even
                    // where the user's refspecs map pull refs.
                    "--refmap=",
                    &remote,
                    &spec,
                ],
                FETCH_TIMEOUT,
                Some(cancel),
            );
            match fetched {
                Err(GitError::Cancelled) => return Err(WorktreeError::Cancelled),
                Err(e) => failed = Some(e),
                Ok(()) => {
                    clone.forget_commit(head);
                    if clone.has_commit(head) {
                        return Ok(true);
                    }
                }
            }
        }
        Err(failed.map_or(WorktreeError::NotServed, WorktreeError::Fetch))
    }

    /// The remote whose URL is the review's repository (`git remote -v`,
    /// fetch URLs), else `origin`.
    fn remote_for(&self, host: &str, repo: &str) -> String {
        let want = format!("{}/{}", strip_port(host), repo.trim_matches('/')).to_lowercase();
        let listed = self
            .read(&["remote", "-v"])
            .map(|o| String::from_utf8_lossy(&o).into_owned())
            .unwrap_or_default();
        listed
            .lines()
            .filter_map(|l| {
                let (name, rest) = l.split_once('\t')?;
                let url = rest.strip_suffix(" (fetch)")?;
                Some((name, url))
            })
            .find(|(name, url)| {
                safe_remote_name(name) && normalize_remote_url(url).as_deref() == Some(&want)
            })
            .map_or_else(|| "origin".to_string(), |(name, _)| name.to_string())
    }

    /// `/.argus/` in the clone's `info/exclude` (the common git dir's, so
    /// it holds in every worktree), once.
    fn exclude_argus(&self) -> Result<(), WorktreeError> {
        let common = self
            .read(&["rev-parse", "--git-common-dir"])
            .map(|o| String::from_utf8_lossy(&o).trim().to_string())
            .filter(|c| !c.is_empty())
            .ok_or(WorktreeError::Setup("the clone's info/exclude"))?;
        let common = Path::new(&common);
        let common = if common.is_absolute() {
            common.to_path_buf()
        } else {
            self.dir.join(common)
        };
        add_exclude(&common.join("info").join("exclude"))
            .map_err(|_| WorktreeError::Setup("the clone's info/exclude"))
    }

    /// Remove the marked worktrees under the root that no run of this
    /// process uses. How many went.
    fn sweep(&self) -> usize {
        let root = self.root();
        let Ok(entries) = std::fs::read_dir(&root) else {
            return 0;
        };
        let in_use = live().lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut removed = 0;
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name();
            let Some(slug) = name
                .to_str()
                .and_then(|n| n.strip_suffix(&format!(".{SIDECAR_EXT}")))
            else {
                continue;
            };
            let path = root.join(slug);
            if !valid_slug(slug) || !is_marker(&entry.path()) || in_use.contains(&path) {
                continue;
            }
            self.remove(&path, true);
            removed += 1;
        }
        removed
    }
}

/// `<path>.argus`, the sidecar of the worktree at `path`.
fn sidecar(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{SIDECAR_EXT}"));
    path.with_file_name(name)
}

/// Whether the file at `path` is a sidecar Argus wrote.
fn is_marker(path: &Path) -> bool {
    let is_file = path
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_file());
    is_file && std::fs::read_to_string(path).is_ok_and(|t| t.lines().next() == Some(MARKER))
}

/// Add [`EXCLUDE_LINE`] to the exclude file at `file` unless it already
/// excludes `.argus/`.
pub(crate) fn add_exclude(file: &Path) -> std::io::Result<()> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let present = text
        .lines()
        .map(str::trim)
        .any(|l| matches!(l, ".argus" | ".argus/" | "/.argus" | "/.argus/"));
    if present {
        return Ok(());
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut out = text;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("# Argus' review worktrees\n");
    out.push_str(EXCLUDE_LINE);
    out.push('\n');
    std::fs::write(file, out)
}

/// The ref the platform keeps a review's head under.
pub(crate) fn head_ref(review: &ReviewRef) -> String {
    match review.provider {
        ProviderId::Github => format!("refs/pull/{}/head", review.number),
        ProviderId::Gitlab => format!("refs/merge-requests/{}/head", review.number),
    }
}

/// `<provider>-<number>-<short sha>-<run>`: unique per run.
fn slug(review: &ReviewRef, head: &str) -> String {
    let run = uuid::Uuid::new_v4().simple().to_string();
    format!(
        "{}-{}-{}-{}",
        review.provider.as_str(),
        review.number,
        &head[..7],
        &run[..8]
    )
}

/// Whether `s` is a name [`slug`] makes.
fn valid_slug(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    let hex = |p: &str, n: usize| p.len() == n && p.bytes().all(|b| b.is_ascii_hexdigit());
    matches!(
        parts.as_slice(),
        [provider, number, sha, run]
            if matches!(*provider, "github" | "gitlab")
                && !number.is_empty()
                && number.bytes().all(|b| b.is_ascii_digit())
                && hex(sha, 7)
                && hex(run, 8)
    )
}

/// A remote name git cannot read as an option.
fn safe_remote_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
}

pub(crate) fn strip_port(host: &str) -> &str {
    if host.starts_with('[') {
        return host.split_inclusive(']').next().unwrap_or(host);
    }
    host.split_once(':').map_or(host, |(h, _)| h)
}

/// A remote URL as `host/path`, lower-cased, for comparing with a review's
/// `host/repo`: scheme, user (and any token in it), port and `.git` left
/// out. `https://u:t@gitlab.example.test:8443/a/b.git` and
/// `git@gitlab.example.test:a/b` are both `gitlab.example.test/a/b`. `None`
/// for a local path.
pub(crate) fn normalize_remote_url(url: &str) -> Option<String> {
    let url = url.trim();
    let (host, path) = if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        (authority.rsplit('@').next()?, path)
    } else {
        // scp-like `user@host:path`: a colon before any slash.
        let colon = url.find(':')?;
        if url[..colon].contains('/') {
            return None;
        }
        (url[..colon].rsplit('@').next()?, &url[colon + 1..])
    };
    let host = strip_port(host);
    let path = path.trim_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{host}/{path}").to_lowercase())
}

impl LocalClone {
    fn repo(&self) -> Repo {
        Repo {
            git: self.git.clone(),
            dir: self.dir.clone(),
        }
    }

    /// A worktree of `review` at `head` for one AI run: the head fetched
    /// when missing, stale worktrees of earlier runs swept first. Removed
    /// when the returned guard drops.
    pub fn review_worktree(
        &self,
        review: &ReviewRef,
        head: &str,
        cancel: &AtomicBool,
    ) -> Result<ReviewWorktree, WorktreeError> {
        if !is_sha(head) || head.len() < 7 {
            return Err(WorktreeError::BadHead);
        }
        let repo = self.repo();
        let lock = clone_lock(&repo.dir);
        let _one_at_a_time = lock.lock().unwrap_or_else(|e| e.into_inner());
        repo.sweep();
        let fetched = repo.ensure_head(self, review, head, cancel)?;
        repo.exclude_argus()?;
        let root = repo.root();
        std::fs::create_dir_all(&root).map_err(|_| WorktreeError::Setup(WORKTREES_DIR))?;
        let path = root.join(slug(review, head));
        let marker = format!(
            "{MARKER}\nreview {}\nhead {head}\npid {}\n",
            head_ref(review),
            std::process::id()
        );
        std::fs::write(sidecar(&path), marker)
            .map_err(|_| WorktreeError::Setup("the worktree's marker"))?;
        live()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(path.clone());
        let p = path.to_string_lossy();
        let added = repo.write_ok(
            &["worktree", "add", "--detach", &p, head],
            ADD_TIMEOUT,
            Some(cancel),
        );
        if let Err(e) = added {
            repo.remove(&path, true);
            live()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&path);
            return Err(match e {
                GitError::Cancelled => WorktreeError::Cancelled,
                e => WorktreeError::Add(e),
            });
        }
        Ok(ReviewWorktree {
            repo,
            path,
            head: head.to_string(),
            fetched,
        })
    }

    /// Remove the review worktrees earlier runs left behind (Argus quit or
    /// crashed mid-run): only marked ones no run of this process uses. How
    /// many went.
    pub fn sweep_review_worktrees(&self) -> usize {
        let repo = self.repo();
        if !repo.root().is_dir() {
            return 0;
        }
        let lock = clone_lock(&repo.dir);
        let _one_at_a_time = lock.lock().unwrap_or_else(|e| e.into_inner());
        repo.sweep()
    }
}

#[cfg(test)]
#[path = "worktree_tests.rs"]
mod tests;
