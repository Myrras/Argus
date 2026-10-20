//! A configured local clone ([`Settings::clone_paths`](crate::settings::Settings::clone_paths)),
//! read through plain `git` to follow thread history and to read files
//! without the platform API: faster, and exact (git's own diffs, two-dot
//! from the exact commit, never truncated).
//!
//! The clone is an accelerator, never a requirement: whatever it cannot
//! answer exactly (a commit not fetched, a shallow boundary, a type change,
//! a file that is not UTF-8, any git failure) is answered by the API
//! instead, per thread or per file. See [`file_or`] and the walk in
//! [`history::api`](crate::history::api).
//!
//! Read-only by construction: only object reads run (`rev-parse --verify`
//! (also of `HEAD`, for opening a file in an editor),
//! `merge-base` (`--is-ancestor` too), `rev-list`, `diff-tree`, `ls-tree`,
//! `cat-file --batch`, `remote -v`; no checkout, no fetch, no status), with
//! `GIT_OPTIONAL_LOCKS=0` so no read takes the index lock, lazy fetches of
//! a partial clone off, and no shell, a timeout, and an environment without
//! tokens or `GIT_*` redirections.
//!
//! The one exception is [`worktree`]: before an AI run it fetches the
//! review's head commit when the clone lacks it and checks it out in a
//! worktree of Argus' own under `<clone>/.argus/worktrees`, removed after
//! the run. Nothing else in the clone changes.

mod diff;
pub mod worktree;

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use crate::error::{CoreError, CoreResult};
use crate::history::LineHistorySource;
use crate::provider::FileDiff;

/// How long one git command may take. Object reads are milliseconds; this
/// only bounds a wedged process (a network filesystem, a huge diff).
const TIMEOUT: Duration = Duration::from_secs(30);

/// Most of stdout kept from one command; more is refused (the API answers).
const MAX_OUTPUT: usize = 64 * 1024 * 1024;

/// Where `git` usually lives when the login shell cannot tell us. The
/// Apple stub `/usr/bin/git` last: without the Command Line Tools it opens
/// an install dialog.
#[cfg(unix)]
const WELL_KNOWN_GIT: &[&str] = &[
    "/opt/homebrew/bin/git",
    "/usr/local/bin/git",
    "/opt/local/bin/git",
    "/usr/bin/git",
];
#[cfg(windows)]
const WELL_KNOWN_GIT: &[&str] = &[];

/// The first git that honours `GIT_NO_LAZY_FETCH`. An older git reading a
/// partial clone could fetch a missing blob, so such a clone is not used.
const NO_LAZY_FETCH_SINCE: (u32, u32) = (2, 44);

/// A resolved `git`.
#[derive(Debug, Clone)]
struct GitBinary {
    path: PathBuf,
    /// (major, minor); (0, 0) when `--version` could not be read.
    version: (u32, u32),
}

/// `git`, resolved once like `gh` (login shell `command -v`, then the
/// well-known places); `None` when it is not installed.
fn git_binary() -> Option<&'static GitBinary> {
    static GIT: OnceLock<Option<GitBinary>> = OnceLock::new();
    GIT.get_or_init(|| {
        let path = PathBuf::from(crate::platform::resolve_binary("git", WELL_KNOWN_GIT)?);
        let version = run(&path, None, &["--version"], None, TIMEOUT)
            .ok()
            .filter(|o| o.code == Some(0))
            .and_then(|o| parse_version(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or((0, 0));
        Some(GitBinary { path, version })
    })
    .as_ref()
}

/// `git version 2.39.5 (Apple Git-154)` → (2, 39).
fn parse_version(out: &str) -> Option<(u32, u32)> {
    let v = out.trim().strip_prefix("git version ")?;
    let mut parts = v.split(|c: char| !c.is_ascii_digit());
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// Whether `s` can only be a full or abbreviated object id. Everything git
/// is handed as a revision passes this first, so nothing a platform sends
/// can read as an option or a revision expression.
fn is_sha(s: &str) -> bool {
    (7..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// One remembered clone check: whether the clone is partial, and the
/// modification time of its config when checked.
type Checked = (Option<SystemTime>, bool);

/// Clones already checked to be usable, by (git, dir). Positives only: a
/// path that was not a repository may become one. A check stands only
/// while the clone's config is unchanged (see [`config_stamp`]): the same
/// directory re-cloned with a filter is partial now.
fn checked() -> &'static Mutex<HashMap<(PathBuf, PathBuf), Checked>> {
    static CHECKED: OnceLock<Mutex<HashMap<(PathBuf, PathBuf), Checked>>> = OnceLock::new();
    CHECKED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// When `dir`'s `.git/config` last changed: cloning (again), or making a
/// clone partial, rewrites it. `None` when there is no such file (a
/// worktree's `.git` is a file pointing elsewhere): checked on every open.
fn config_stamp(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir.join(".git").join("config"))
        .ok()
        .filter(|m| m.is_file())?
        .modified()
        .ok()
}

/// What a git command returned.
#[derive(Debug)]
struct Output {
    code: Option<i32>,
    stdout: Vec<u8>,
}

/// Why a git command gave no output. Argus' own text; git's stderr is
/// never read, so nothing it prints reaches a log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitError {
    Spawn,
    Timeout,
    /// The run it was part of was cancelled, and git was stopped.
    Cancelled,
    TooLarge,
    Exit(Option<i32>),
    TypeChange,
    Malformed,
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::Spawn => write!(f, "git could not be started"),
            GitError::Timeout => write!(f, "git did not finish in time and was stopped"),
            GitError::Cancelled => write!(f, "cancelled"),
            GitError::TooLarge => write!(f, "git's output is too large"),
            GitError::Exit(Some(c)) => write!(f, "git exited with status {c}"),
            GitError::Exit(None) => write!(f, "git was killed"),
            GitError::TypeChange => write!(f, "a file changed type"),
            GitError::Malformed => write!(f, "git's output could not be read"),
        }
    }
}

impl From<diff::DiffError> for GitError {
    fn from(e: diff::DiffError) -> Self {
        match e {
            diff::DiffError::TypeChange => GitError::TypeChange,
            diff::DiffError::Malformed => GitError::Malformed,
        }
    }
}

impl From<GitError> for CoreError {
    fn from(e: GitError) -> Self {
        CoreError::Provider(format!("the local clone could not be read ({e})"))
    }
}

/// The environment git runs with: Argus' own minus every token variable
/// and every `GIT_*` one (a `GIT_DIR` or `GIT_INDEX_FILE` inherited from a
/// hook would point git elsewhere), plus what keeps it read-only and
/// quiet.
pub(crate) fn git_env(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    let is_git = |k: &OsStr| k.to_string_lossy().to_ascii_uppercase().starts_with("GIT_");
    let mut out: Vec<(OsString, OsString)> = vars
        .into_iter()
        .filter(|(k, _)| !crate::ai::adapter::is_token_var(k) && !is_git(k))
        .collect();
    for (k, v) in [
        // Reads never take `index.lock` (no index refresh behind the
        // user's back, no clash with their own git).
        ("GIT_OPTIONAL_LOCKS", "0"),
        // A partial clone's missing object is not fetched (git ≥ 2.44).
        ("GIT_NO_LAZY_FETCH", "1"),
        // History as the platforms see it, not as local replace refs say.
        ("GIT_NO_REPLACE_OBJECTS", "1"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_PAGER", "cat"),
    ] {
        out.push((k.into(), v.into()));
    }
    out
}

#[cfg(windows)]
fn hide_console_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
fn hide_console_window(_cmd: &mut Command) {}

/// Run `git [-C dir] <args>` (no shell), feeding `stdin`, killed after
/// `timeout`, with [`git_env`] of Argus' own environment.
fn run(
    git: &Path,
    dir: Option<&Path>,
    args: &[&str],
    stdin: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<Output, GitError> {
    run_with(
        git,
        dir,
        args,
        stdin,
        timeout,
        &git_env(std::env::vars_os()),
        None,
    )
}

/// How often a cancellable git command checks its run's cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(50);

/// [`run`] with the complete environment `env`, and stopped (killed) as
/// soon as `cancel` is set.
fn run_with(
    git: &Path,
    dir: Option<&Path>,
    args: &[&str],
    stdin: Option<Vec<u8>>,
    timeout: Duration,
    env: &[(OsString, OsString)],
    cancel: Option<&AtomicBool>,
) -> Result<Output, GitError> {
    let cancelled = || cancel.is_some_and(|c| c.load(Ordering::Relaxed));
    if cancelled() {
        return Err(GitError::Cancelled);
    }
    let mut cmd = Command::new(git);
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
    }
    cmd.args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide_console_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|_| GitError::Spawn)?;
    // Written from its own thread: input larger than the pipe buffer would
    // otherwise deadlock against git already writing stdout.
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = pipe.write_all(&input);
        });
    }
    let stdout = child.stdout.take().expect("stdout piped");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let read = stdout
            .take(MAX_OUTPUT as u64 + 1)
            .read_to_end(&mut buf)
            .map(|_| buf);
        let _ = tx.send(read);
    });
    let deadline = Instant::now() + timeout;
    let stop = |child: &mut std::process::Child, e: GitError| {
        let _ = child.kill();
        let _ = child.wait();
        Err(e)
    };
    // Waiting on the reader, not polling the process: a millisecond
    // command returns in milliseconds. A cancellable one wakes up now and
    // then to look at its flag.
    let stdout = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let slice = if cancel.is_some() {
            left.min(CANCEL_POLL)
        } else {
            left
        };
        match rx.recv_timeout(slice) {
            Ok(Ok(buf)) if buf.len() <= MAX_OUTPUT => break buf,
            Ok(Ok(_)) => return stop(&mut child, GitError::TooLarge),
            Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                return stop(&mut child, GitError::Spawn)
            }
            Err(mpsc::RecvTimeoutError::Timeout) if cancelled() => {
                return stop(&mut child, GitError::Cancelled)
            }
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                return stop(&mut child, GitError::Timeout)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    // stdout is closed: git is exiting.
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(Output {
                    code: status.code(),
                    stdout,
                })
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1));
            }
            _ => return stop(&mut child, GitError::Timeout),
        }
    }
}

/// A usable local clone. Cheap to open (the check is remembered per path
/// while its config is unchanged);
/// the commit lookups and diffs it reads are memoised for its lifetime, so
/// open one per computation and share it between the threads of a review.
pub struct LocalClone {
    git: PathBuf,
    dir: PathBuf,
    timeout: Duration,
    /// A partial clone (objects may be missing locally): a path that does
    /// not resolve is then not proof that the commit has no such file.
    partial: bool,
    /// Whether a commit is in the clone, as asked so far.
    commits: Mutex<HashMap<String, bool>>,
    /// Commit diffs against their first parent, by commit.
    diffs: Mutex<HashMap<String, FileDiff>>,
}

impl std::fmt::Debug for LocalClone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalClone")
            .field("dir", &self.dir)
            .finish()
    }
}

impl LocalClone {
    /// The clone at `dir`, when it is a git repository Argus can read
    /// without side effects; `None` otherwise (git missing, not a
    /// repository, a partial clone with a git too old to stop lazy
    /// fetches), and the API is used. The reason is logged at debug level.
    pub fn open(dir: &Path) -> Option<LocalClone> {
        let Some(git) = git_binary() else {
            tracing::debug!("local clone not used: git is not installed");
            return None;
        };
        Self::open_with(&git.path, git.version, dir, TIMEOUT)
    }

    fn open_with(
        git: &Path,
        version: (u32, u32),
        dir: &Path,
        timeout: Duration,
    ) -> Option<LocalClone> {
        if !dir.is_dir() {
            tracing::debug!("local clone not used: {} is not a directory", dir.display());
            return None;
        }
        let mut clone = LocalClone {
            git: git.to_path_buf(),
            dir: dir.to_path_buf(),
            timeout,
            partial: false,
            commits: Mutex::new(HashMap::new()),
            diffs: Mutex::new(HashMap::new()),
        };
        let key = (git.to_path_buf(), dir.to_path_buf());
        let stamp = config_stamp(dir);
        let known = checked()
            .lock()
            .expect("clone check lock")
            .get(&key)
            .copied()
            .filter(|(at, _)| at.is_some() && *at == stamp);
        clone.partial = match known {
            Some((_, partial)) => partial,
            None => match clone.check(version) {
                Ok(partial) => {
                    checked()
                        .lock()
                        .expect("clone check lock")
                        .insert(key, (stamp, partial));
                    partial
                }
                Err(why) => {
                    tracing::debug!("local clone {} not used: {why}", dir.display());
                    return None;
                }
            },
        };
        Some(clone)
    }

    /// Whether the directory is a repository that can be read safely, and
    /// whether it is a partial clone.
    fn check(&self, version: (u32, u32)) -> Result<bool, String> {
        let out = self
            .git(&["rev-parse", "--git-dir"], None)
            .map_err(|e| e.to_string())?;
        if out.code != Some(0) {
            return Err("not a git repository".into());
        }
        // Older partial clones set `extensions.partialClone`, newer ones a
        // promisor remote.
        let config = self
            .git(
                &[
                    "config",
                    "--get-regexp",
                    r"^(extensions\.partialclone|remote\..*\.promisor)$",
                ],
                None,
            )
            .map_err(|e| e.to_string())?;
        let partial = String::from_utf8_lossy(&config.stdout).lines().any(|l| {
            let (key, value) = l.split_once(' ').unwrap_or((l, ""));
            key == "extensions.partialclone"
                || matches!(
                    value.to_ascii_lowercase().as_str(),
                    "true" | "yes" | "on" | "1"
                )
        });
        if partial && version < NO_LAZY_FETCH_SINCE {
            return Err("a partial clone, and this git could fetch missing objects".into());
        }
        Ok(partial)
    }

    fn git(&self, args: &[&str], stdin: Option<Vec<u8>>) -> Result<Output, GitError> {
        run(&self.git, Some(&self.dir), args, stdin, self.timeout)
    }

    /// `git <args>` that must exit 0; its stdout.
    fn git_ok(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        let out = self.git(args, None)?;
        match out.code {
            Some(0) => Ok(out.stdout),
            code => Err(GitError::Exit(code)),
        }
    }

    /// Whether `sha` is a commit in the clone (without fetching anything).
    pub fn has_commit(&self, sha: &str) -> bool {
        if !is_sha(sha) {
            return false;
        }
        if let Some(&known) = self.commits.lock().expect("commits lock").get(sha) {
            return known;
        }
        let present = self
            .git(
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("{sha}^{{commit}}"),
                ],
                None,
            )
            .is_ok_and(|o| o.code == Some(0));
        self.commits
            .lock()
            .expect("commits lock")
            .insert(sha.to_string(), present);
        present
    }

    /// Forget what [`has_commit`](Self::has_commit) found for `sha`: after
    /// a fetch, a commit that was missing may be here.
    pub(crate) fn forget_commit(&self, sha: &str) {
        self.commits.lock().expect("commits lock").remove(sha);
    }

    /// The commit the remote's default branch points at
    /// (`refs/remotes/origin/HEAD`), else the clone's own `HEAD`: where a
    /// listing without a review reads the project's skills from.
    pub fn default_branch_head(&self) -> Option<String> {
        for rev in ["refs/remotes/origin/HEAD^{commit}", "HEAD^{commit}"] {
            if let Ok(out) = self.git_ok(&["rev-parse", "--verify", "--quiet", rev]) {
                let sha = String::from_utf8_lossy(&out).trim().to_string();
                if is_sha(&sha) {
                    return Some(sha);
                }
            }
        }
        None
    }

    /// The entries of the directory `dir` (`""`: the root) at `sha`, as
    /// (name, is a directory), from the object database (`ls-tree`).
    /// Empty when the commit has no such directory; `None` when the commit
    /// is not here or git failed.
    pub fn list_dir(&self, sha: &str, dir: &str) -> Option<Vec<(String, bool)>> {
        if !self.has_commit(sha) || dir.contains(['\n', '\0']) {
            return None;
        }
        let dir = dir.trim_matches('/');
        let mut args = vec!["ls-tree", "-z", sha];
        let spec = format!("{dir}/");
        if !dir.is_empty() {
            args.extend(["--", spec.as_str()]);
        }
        let out = self.git_ok(&args).ok()?;
        Some(
            out.split(|b| *b == 0)
                .filter_map(|entry| {
                    // `<mode> SP <type> SP <oid> TAB <path>`
                    let entry = std::str::from_utf8(entry).ok()?;
                    let (meta, path) = entry.split_once('\t')?;
                    let kind = meta.split(' ').nth(1)?;
                    let name = path.rsplit('/').next()?.to_string();
                    Some((name, kind == "tree"))
                })
                .collect(),
        )
    }

    /// The contents of each of `paths` at `sha`, read in one `cat-file
    /// --batch`: `None` for a path that is no UTF-8 file there. `None` as a
    /// whole when the commit is not here or git failed.
    pub fn files_at(&self, sha: &str, paths: &[String]) -> Option<Vec<Option<String>>> {
        if !is_sha(sha)
            || paths
                .iter()
                .any(|p| p.is_empty() || p.contains(['\n', '\0']))
        {
            return None;
        }
        let mut input = format!("{sha}^{{commit}}\n");
        for p in paths {
            input.push_str(&format!("{sha}:{p}\n"));
        }
        let out = self
            .git(&["cat-file", "--batch"], Some(input.into_bytes()))
            .ok()
            .filter(|o| o.code == Some(0))?
            .stdout;
        let mut rest = out.as_slice();
        if batch_record(&mut rest)?.0 != BatchKind::Object("commit") {
            return None;
        }
        let mut files = Vec::with_capacity(paths.len());
        for _ in paths {
            files.push(match batch_record(&mut rest)? {
                (BatchKind::Object("blob"), content) => String::from_utf8(content.to_vec()).ok(),
                _ => None,
            });
        }
        Some(files)
    }

    /// The paths changed from `from` to `to` (two-dot; a rename as its new
    /// path and its old one). `None` when either commit is not here or git
    /// failed.
    pub fn changed_paths(&self, from: &str, to: &str) -> Option<Vec<String>> {
        if !self.has_commit(from) || !self.has_commit(to) {
            return None;
        }
        let out = self
            .git_ok(&[
                "diff-tree",
                "-r",
                "-z",
                "--name-only",
                "--no-renames",
                from,
                to,
            ])
            .ok()?;
        Some(
            out.split(|b| *b == 0)
                .filter(|p| !p.is_empty())
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .collect(),
        )
    }

    /// The commit the clone has checked out (`git rev-parse --verify HEAD`,
    /// a ref read like the others); `None` on an unborn branch or when git
    /// failed.
    pub fn head(&self) -> Option<String> {
        let out = self
            .git_ok(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
            .ok()?;
        let sha = String::from_utf8(out).ok()?.trim().to_string();
        is_sha(&sha).then_some(sha)
    }

    /// The merge base of commits `a` and `b` (`git merge-base`, an object
    /// read like the others); `None` when either is no sha or not in the
    /// clone, there is none, or git failed.
    pub fn merge_base(&self, a: &str, b: &str) -> Option<String> {
        if !self.has_commit(a) || !self.has_commit(b) {
            return None;
        }
        let out = self.git_ok(&["merge-base", a, b]).ok()?;
        let sha = String::from_utf8(out).ok()?.trim().to_string();
        is_sha(&sha).then_some(sha)
    }

    /// The content of `path` at `sha`, read from the object database (not
    /// the working tree: local edits never show). `None` when the clone
    /// cannot answer exactly (the commit is not here, the path is not a
    /// file, the content is not UTF-8, git failed) and the API should;
    /// `Some(Err(NotFound))` when the commit is here and has no such file
    /// (not for a partial clone, where the object may just not be local).
    pub fn file_at(&self, sha: &str, path: &str) -> Option<CoreResult<String>> {
        if !is_sha(sha) || path.is_empty() || path.contains(['\n', '\0']) {
            return None;
        }
        // One process for both: is the commit here, and the blob. A whole
        // input line is the object name, so spaces in `path` are fine.
        let input = format!("{sha}^{{commit}}\n{sha}:{path}\n");
        let out = match self.git(&["cat-file", "--batch"], Some(input.into_bytes())) {
            Ok(o) if o.code == Some(0) => o.stdout,
            Ok(o) => {
                tracing::debug!("local clone: cat-file failed ({})", GitError::Exit(o.code));
                return None;
            }
            Err(e) => {
                tracing::debug!("local clone: cat-file failed ({e})");
                return None;
            }
        };
        let mut rest = out.as_slice();
        let (kind, _) = batch_record(&mut rest)?;
        let commit_here = kind == BatchKind::Object("commit");
        self.commits
            .lock()
            .expect("commits lock")
            .insert(sha.to_string(), commit_here);
        if !commit_here {
            tracing::debug!("local clone lacks commit {sha}; reading the file through the API");
            return None;
        }
        match batch_record(&mut rest)? {
            (BatchKind::Missing, _) if !self.partial => Some(Err(CoreError::NotFound)),
            (BatchKind::Object("blob"), content) => {
                String::from_utf8(content.to_vec()).ok().map(Ok)
            }
            _ => None,
        }
    }

    /// The diff from `base` to `head` (two-dot: exactly from `base`).
    fn diff(&self, base: &str, head: &str) -> Result<FileDiff, GitError> {
        let raw = self.git_ok(&[
            "diff-tree",
            "-r",
            "-M",
            "-z",
            "--raw",
            "--no-abbrev",
            base,
            head,
        ])?;
        let raw = diff::parse_raw(&raw)?;
        let patch = self.git_ok(&[
            "diff-tree",
            "-r",
            "-M",
            "-p",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            base,
            head,
        ])?;
        Ok(diff::assemble(
            base,
            head,
            raw,
            &String::from_utf8_lossy(&patch),
        )?)
    }

    fn checked_sha<'s>(&self, sha: &'s str) -> CoreResult<&'s str> {
        if is_sha(sha) {
            Ok(sha)
        } else {
            Err(CoreError::InvalidInput("not a commit sha".into()))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum BatchKind<'a> {
    Object(&'a str),
    Missing,
    /// `ambiguous` and anything else git may answer.
    Other,
}

/// One record of `cat-file --batch` output, advancing `rest` past it:
/// `<oid> <type> <size>\n<content>\n`, or `<name> missing\n`.
fn batch_record<'a>(rest: &mut &'a [u8]) -> Option<(BatchKind<'a>, &'a [u8])> {
    let nl = rest.iter().position(|b| *b == b'\n')?;
    let header = std::str::from_utf8(&rest[..nl]).ok()?;
    *rest = &rest[nl + 1..];
    if header.ends_with(" missing") {
        return Some((BatchKind::Missing, &[]));
    }
    let mut parts = header.rsplitn(3, ' ');
    let (Some(size), Some(kind), Some(_oid)) = (parts.next(), parts.next(), parts.next()) else {
        return Some((BatchKind::Other, &[]));
    };
    let Ok(size) = size.parse::<usize>() else {
        return Some((BatchKind::Other, &[]));
    };
    if rest.len() < size + 1 {
        return None;
    }
    let content = &rest[..size];
    *rest = &rest[size + 1..];
    Some((BatchKind::Object(kind), content))
}

impl LineHistorySource for LocalClone {
    /// git's own first-parent line from `from` to `to`; `None` when `from`
    /// is not an ancestor of `to` (rebased away), and the walk takes the
    /// direct diff, which the clone can still give exactly.
    fn commits_between(&self, from: &str, to: &str) -> CoreResult<Option<Vec<String>>> {
        let (from, to) = (self.checked_sha(from)?, self.checked_sha(to)?);
        let ancestor = self.git(&["merge-base", "--is-ancestor", from, to], None)?;
        match ancestor.code {
            Some(0) => {}
            Some(1) => return Ok(None),
            code => return Err(GitError::Exit(code).into()),
        }
        let range = format!("{from}..{to}");
        let list = self.git_ok(&["rev-list", "--reverse", "--first-parent", &range])?;
        Ok(Some(
            String::from_utf8_lossy(&list)
                .lines()
                .map(str::to_string)
                .collect(),
        ))
    }

    fn commit_diff(&self, sha: &str) -> CoreResult<FileDiff> {
        let sha = self.checked_sha(sha)?;
        if let Some(d) = self.diffs.lock().expect("diffs lock").get(sha) {
            return Ok(d.clone());
        }
        let parent = self.git_ok(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{sha}^1^{{commit}}"),
        ])?;
        let parent = String::from_utf8_lossy(&parent).trim().to_string();
        // Not held across git: threads walking other commits go on.
        let diff = self.diff(&parent, sha)?;
        self.diffs
            .lock()
            .expect("diffs lock")
            .insert(sha.to_string(), diff.clone());
        Ok(diff)
    }

    fn direct_diff(&self, from: &str, to: &str) -> CoreResult<Option<FileDiff>> {
        let (from, to) = (self.checked_sha(from)?, self.checked_sha(to)?);
        Ok(Some(self.diff(from, to)?))
    }
}

/// The content of `path` at `sha`: from `clone` when it can answer, else
/// from `fallback` (the provider).
pub(crate) fn file_or(
    clone: Option<&LocalClone>,
    sha: &str,
    path: &str,
    fallback: impl FnOnce() -> CoreResult<String>,
) -> CoreResult<String> {
    match clone.and_then(|c| c.file_at(sha, path)) {
        Some(answer) => answer,
        None => fallback(),
    }
}

#[cfg(test)]
pub(crate) mod tests;
