//! Looking for a clone of a review's repository on disk, for a repository
//! that has none set yet (see [`Core::suggest_clone`](crate::Core::suggest_clone)).
//!
//! Where: the folders that already hold the user's other clones (a
//! configured clone's parent), plus `~/code`, `~/src` and `~/Developer`.
//! At most [`MAX_DEPTH`] levels under each, the usual build and dependency
//! folders skipped, symlinks never followed (a link back up would walk for
//! ever) and at most [`MAX_DIRS`] directories looked at, so a scan stays a
//! moment's work however deep the folders go.
//!
//! What counts as a clone of the repository: a directory with a `.git`
//! whose `git remote -v` names the review's `host/repo`, compared with the
//! same [`normalize_remote_url`] the worktree code picks a fetch remote
//! with. Only object-free reads run (`remote -v`), through the read-only
//! [`run`] of this module: nothing is fetched, nothing is written.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::worktree::{normalize_remote_url, strip_port};
use super::{git_binary, run, TIMEOUT};

/// How deep under a search root a clone is looked for: `~/code/acme/widgets`
/// is three levels, which is as far as a clone is normally filed.
pub(crate) const MAX_DEPTH: usize = 3;

/// At most this many directories per scan: a bound on an unusually wide
/// tree, so the scan cannot run on and on.
const MAX_DIRS: usize = 4000;

/// Never descended into: dependency and build folders (huge, never a clone
/// of the review), and the place a macOS home keeps application data.
/// Hidden folders (`.git`, `.cache`, `.venv`, Argus' `.argus`) are skipped
/// by [`children`] already.
const SKIP: &[&str] = &[
    "Library",
    "Pods",
    "__pycache__",
    "bower_components",
    "build",
    "dist",
    "node_modules",
    "target",
    "vendor",
    "venv",
];

/// The folders under the home directory that are tried besides the parents
/// of the clones already configured.
const HOME_ROOTS: &[&str] = &["code", "src", "Developer"];

/// The user's home directory, as git itself reads it.
pub(crate) fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// Where to look: the parent of every configured clone, then the home
/// folders of [`HOME_ROOTS`]. Only existing directories, each once, in that
/// order (the user's own filing habit first).
pub(crate) fn search_roots<'a>(
    clone_paths: impl IntoIterator<Item = &'a Path>,
    home: Option<&Path>,
) -> Vec<PathBuf> {
    let candidates = clone_paths
        .into_iter()
        .filter_map(|p| p.parent().map(Path::to_path_buf))
        .chain(
            home.into_iter()
                .flat_map(|h| HOME_ROOTS.iter().map(|r| h.join(r))),
        );
    let mut seen = HashSet::new();
    candidates
        .filter(|p| p.is_dir())
        .filter_map(|p| std::fs::canonicalize(p).ok())
        .filter(|p| seen.insert(p.clone()))
        .collect()
}

/// `host/repo` as [`normalize_remote_url`] spells it, for comparing.
fn want(host: &str, repo: &str) -> String {
    format!("{}/{}", strip_port(host), repo.trim_matches('/')).to_lowercase()
}

/// Whether the repository at `dir` has a remote pointing at `host/repo`.
/// Fetch and push URLs both count: a fork set up as `push` only is still
/// that repository's clone. Any git failure is a no.
pub(crate) fn is_clone_of(host: &str, repo: &str, dir: &Path) -> bool {
    let Some(git) = git_binary() else {
        return false;
    };
    has_remote(&git.path, dir, &want(host, repo))
}

fn has_remote(git: &Path, dir: &Path, want: &str) -> bool {
    let Ok(out) = run(git, Some(dir), &["remote", "-v"], None, TIMEOUT) else {
        return false;
    };
    if out.code != Some(0) {
        return false;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            // `<name> TAB <url> SP (fetch|push)`
            let (_, rest) = l.split_once('\t')?;
            let url = rest.rsplit_once(' ').map(|(url, _)| url)?;
            normalize_remote_url(url)
        })
        .any(|url| url == want)
}

/// The first clone of `host/repo` found under `roots` (breadth first, so a
/// clone filed shallowly wins over one buried deeper); `None` when there is
/// none, or git is not installed.
pub(crate) fn find_clone(host: &str, repo: &str, roots: &[PathBuf]) -> Option<PathBuf> {
    let git = &git_binary()?.path;
    let want = want(host, repo);
    let mut level: Vec<PathBuf> = roots.to_vec();
    let mut seen: HashSet<PathBuf> = level.iter().cloned().collect();
    let mut looked_at = 0;
    for _ in 0..MAX_DEPTH {
        let mut next = Vec::new();
        for dir in level {
            for child in children(&dir) {
                looked_at += 1;
                if looked_at > MAX_DIRS {
                    tracing::debug!("clone search: stopped after {MAX_DIRS} folders");
                    return None;
                }
                // A repository is answered for, never descended into: its
                // working tree holds no other clone of this review.
                if child.join(".git").exists() {
                    if has_remote(git, &child, &want) {
                        return Some(child);
                    }
                } else if seen.insert(child.clone()) {
                    next.push(child);
                }
            }
        }
        level = next;
    }
    None
}

/// The directories directly in `dir`, sorted by name (so the same tree
/// gives the same answer twice), without the skipped names, hidden folders
/// and symlinks (which could lead back up, or out of the user's disk).
fn children(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.') && !SKIP.contains(&name.as_ref())
        })
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

#[cfg(test)]
#[path = "discover_tests.rs"]
mod tests;
