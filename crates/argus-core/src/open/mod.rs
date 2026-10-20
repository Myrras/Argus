//! Open a review's file from the configured local clone in the user's
//! editor, or show it in the file manager: Iris's `open_local`, keyed by
//! review and repository path instead of an absolute path.
//!
//! The webview never names a local path: it sends the review and the path
//! as the review's diff spells it, and the core resolves that inside the
//! clone set for the review's repository ([`Settings::clone_path`]). A path
//! that is absolute, has a `..` (or `.`) part, or leads out of the clone
//! through a symlink is refused; only a regular file in the working tree is
//! opened.
//!
//! Which application: the one set for the file's extension in
//! `editorOverrides` (the last extension, case-insensitive, as in Iris),
//! else `defaultEditor`, else the OS default. The OS default is refused
//! for a file that would run instead of open (an executable, a `.command`,
//! an `.exe`): the file comes from someone else's change.
//!
//! Launching goes through the injected [`Opener`] (the shell's, over
//! tauri-plugin-opener; tests use a fake), so the core stays Tauri-free.
//! Nothing here writes to the clone: the only git call is `rev-parse HEAD`
//! through [`crate::git::LocalClone`], to tell whether the clone is on the
//! review's head.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::settings::Settings;
use crate::{git, Core, CoreError, CoreResult, ReviewId};

/// Hands a file to an application, or shows it in the file manager.
pub trait Opener: Send + Sync {
    /// Open `path` with `app` (a path to an application or executable), or
    /// with the OS default when `None`. Errors are app-owned text.
    fn open(&self, path: &Path, app: Option<&str>) -> CoreResult<()>;

    /// Show `path` selected in the OS file manager (Finder / Explorer).
    /// Errors are app-owned text.
    fn reveal(&self, path: &Path) -> CoreResult<()>;
}

/// The opener of a core the shell did not set one on.
pub(crate) struct NoOpener;

impl Opener for NoOpener {
    fn open(&self, _path: &Path, _app: Option<&str>) -> CoreResult<()> {
        Err(not_configured())
    }

    fn reveal(&self, _path: &Path) -> CoreResult<()> {
        Err(not_configured())
    }
}

fn not_configured() -> CoreError {
    CoreError::Io(std::io::Error::other("opening files is not set up"))
}

/// A review's file in the local clone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalFile {
    /// The repository path, as asked.
    pub path: String,
    /// Where it is (or would be) in the clone's working tree.
    pub absolute_path: PathBuf,
    /// A regular file is there now (a removed or not yet pulled file, a
    /// folder or a submodule is not).
    pub exists: bool,
    /// The commit the clone has checked out; `None` when git could not
    /// tell (not installed, an unborn branch).
    pub clone_head: Option<String>,
    /// The clone is on the review's head commit, so the file is what the
    /// review shows (local edits aside).
    pub matches_review_head: bool,
}

/// The clone set for a review's repository, as the header chip shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneStatus {
    /// A clone is set for the review's repository in Settings.
    pub configured: bool,
    /// The folder as it is set; `None` when not configured.
    pub path: Option<PathBuf>,
    /// The folder is there and is a git repository.
    pub found: bool,
    /// The commit the clone has checked out; `None` when not found or git
    /// could not tell.
    pub head: Option<String>,
    /// The clone is on the review's head commit.
    pub matches_review_head: bool,
}

/// Where a repository path is in the clone at `root` (canonical).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub path: PathBuf,
    pub is_file: bool,
}

/// Resolve the repository path `rel` inside the clone at `root` (already
/// canonical). Refused: an empty path, an absolute one, `..` or `.` parts,
/// a backslash or NUL, and anything that resolves (through a symlink) to
/// outside `root`.
pub(crate) fn resolve(root: &Path, rel: &str) -> CoreResult<Resolved> {
    let refused = || CoreError::InvalidInput("that path is not inside the local clone".into());
    if rel.is_empty() || rel.contains(['\0', '\\']) {
        return Err(refused());
    }
    let rel_path = Path::new(rel);
    // Only plain names: no root, no drive prefix, no `.` or `..`.
    if !rel_path
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(refused());
    }
    let joined = root.join(rel_path);
    match std::fs::canonicalize(&joined) {
        // Symlinks resolved: the real place must still be in the clone.
        Ok(real) if real.starts_with(root) => {
            let is_file = std::fs::metadata(&real).is_ok_and(|m| m.is_file());
            Ok(Resolved {
                path: real,
                is_file,
            })
        }
        Ok(_) => Err(refused()),
        // Not there (a removed file), or a dangling link: nothing to open.
        Err(_) => Ok(Resolved {
            path: joined,
            is_file: false,
        }),
    }
}

/// The last extension of `path`, lower-case, without the dot (Iris's
/// `extension`): `Notes.MD` → `md`, `x.tar.gz` → `gz`, `.env` → none.
pub(crate) fn extension(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy();
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() => Some(name[dot + 1..].to_lowercase()),
        _ => None,
    }
}

/// The application `settings` open `path` with: its extension's override,
/// else the default editor; `None` is the OS default.
pub(crate) fn editor_for<'s>(settings: &'s Settings, path: &Path) -> Option<&'s str> {
    extension(path)
        .and_then(|ext| settings.editor_overrides.get(&ext))
        .or(settings.default_editor.as_ref())
        .map(String::as_str)
}

/// Extensions the OS runs (or installs, or follows) rather than shows when
/// it opens them with their default application.
const RUNS_ON_OPEN: &[&str] = &[
    "app",
    "command",
    "tool",
    "terminal",
    "workflow",
    "action",
    "scpt",
    "scptd",
    "applescript",
    "pkg",
    "mpkg",
    "dmg",
    "jar",
    "webloc",
    "inetloc",
    "fileloc",
    "url",
    "lnk",
    "exe",
    "com",
    "bat",
    "cmd",
    "msi",
    "msc",
    "scr",
    "pif",
    "reg",
    "hta",
    "cpl",
    "ps1",
    "psm1",
    "vbs",
    "vbe",
    "wsf",
    "wsh",
];

/// Windows hands these to the script host by default: they run.
#[cfg(windows)]
const RUNS_ON_OPEN_WINDOWS: &[&str] = &["js", "jse"];
#[cfg(not(windows))]
const RUNS_ON_OPEN_WINDOWS: &[&str] = &[];

/// Whether the OS default would run `path` instead of showing it: an
/// executable file (macOS opens one in Terminal and runs it) or a
/// [`RUNS_ON_OPEN`] extension.
pub(crate) fn runs_on_open(path: &Path) -> bool {
    if let Some(ext) = extension(path) {
        if RUNS_ON_OPEN.contains(&ext.as_str()) || RUNS_ON_OPEN_WINDOWS.contains(&ext.as_str()) {
            return true;
        }
    }
    is_executable(path)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    false
}

impl Core {
    /// Set what opens files and shows them in the file manager (the
    /// shell's, at startup; tests set a fake).
    pub fn set_opener(&self, opener: Arc<dyn Opener>) {
        *self.opener.lock().expect("opener lock poisoned") = opener;
    }

    fn opener(&self) -> Arc<dyn Opener> {
        self.opener.lock().expect("opener lock poisoned").clone()
    }

    /// The clone set for the review's repository, canonical; `None` when
    /// none is set or the folder is gone.
    fn clone_root(&self, id: ReviewId) -> CoreResult<(crate::ReviewRecord, Option<PathBuf>)> {
        let record = self.review(id)?;
        let root = self
            .settings()
            .clone_path(&record.review)
            .and_then(|dir| std::fs::canonicalize(dir).ok())
            .filter(|dir| dir.is_dir());
        Ok((record, root))
    }

    /// What is set and found for the review's repository: nothing set, a
    /// folder that is gone or no repository, or a repository with its head
    /// (blocking: one `git rev-parse HEAD`). Read-only, like the rest of
    /// this module.
    pub fn clone_status(&self, id: ReviewId) -> CoreResult<CloneStatus> {
        let record = self.review(id)?;
        let settings = self.settings();
        let Some(dir) = settings.clone_path(&record.review) else {
            return Ok(CloneStatus::default());
        };
        let root = std::fs::canonicalize(dir).ok().filter(|d| d.is_dir());
        let clone = root.as_deref().and_then(git::LocalClone::open);
        // `LocalClone::open` also says no for a git that is missing or too
        // old for a partial clone: a `.git` entry still makes it a repository.
        let found = root
            .as_ref()
            .is_some_and(|r| clone.is_some() || r.join(".git").exists());
        let head = clone.and_then(|c| c.head());
        let matches_review_head = head.as_deref() == Some(record.head_sha.as_str());
        Ok(CloneStatus {
            configured: true,
            path: Some(dir.to_path_buf()),
            found,
            head,
            matches_review_head,
        })
    }

    /// Show the review's clone folder in the file manager. Only the folder
    /// the user set for the repository (resolved here, never taken from the
    /// webview).
    pub fn reveal_clone(&self, id: ReviewId) -> CoreResult<()> {
        let (_, root) = self.clone_root(id)?;
        let root = root.ok_or_else(|| {
            CoreError::InvalidInput("no local clone is set for this repository".into())
        })?;
        self.opener().reveal(&root)
    }

    /// The review's files `paths` in its local clone (blocking: one
    /// `git rev-parse HEAD`). `None` when no clone is set for the
    /// repository (or its folder is gone): the UI then offers no editor.
    /// A path the clone cannot hold (absolute, `..`, out through a symlink)
    /// is left out.
    pub fn local_files(
        &self,
        id: ReviewId,
        paths: &[String],
    ) -> CoreResult<Option<Vec<LocalFile>>> {
        let (record, Some(root)) = self.clone_root(id)? else {
            return Ok(None);
        };
        let clone_head = git::LocalClone::open(&root).and_then(|c| c.head());
        let matches_review_head = clone_head.as_deref() == Some(record.head_sha.as_str());
        Ok(Some(
            paths
                .iter()
                .filter_map(|path| match resolve(&root, path) {
                    Ok(r) => Some(LocalFile {
                        path: path.clone(),
                        absolute_path: r.path,
                        exists: r.is_file,
                        clone_head: clone_head.clone(),
                        matches_review_head,
                    }),
                    Err(_) => {
                        tracing::debug!("a review path does not resolve inside the local clone");
                        None
                    }
                })
                .collect(),
        ))
    }

    /// [`local_files`](Self::local_files) for one path; a path the clone
    /// cannot hold is an error.
    pub fn local_file(&self, id: ReviewId, path: &str) -> CoreResult<Option<LocalFile>> {
        let (_, Some(root)) = self.clone_root(id)? else {
            return Ok(None);
        };
        resolve(&root, path)?;
        Ok(self
            .local_files(id, &[path.to_string()])?
            .and_then(|mut v| v.pop()))
    }

    /// The review's file `path` in the working tree, which must be a
    /// regular file.
    fn openable(&self, id: ReviewId, path: &str) -> CoreResult<PathBuf> {
        let (_, root) = self.clone_root(id)?;
        let root = root.ok_or_else(|| {
            CoreError::InvalidInput("no local clone is set for this repository".into())
        })?;
        let r = resolve(&root, path)?;
        if !r.is_file {
            return Err(CoreError::InvalidInput(
                "the file is not in your working tree".into(),
            ));
        }
        Ok(r.path)
    }

    /// Open the review's file `path` from its local clone in the editor
    /// set for it (see the module docs); with `use_default`, in the OS
    /// default application instead. The path is resolved here, never taken
    /// from the webview.
    pub fn open_in_editor(&self, id: ReviewId, path: &str, use_default: bool) -> CoreResult<()> {
        let file = self.openable(id, path)?;
        let settings = self.settings();
        let app = if use_default {
            None
        } else {
            editor_for(&settings, &file)
        };
        if app.is_none() && runs_on_open(&file) {
            return Err(CoreError::InvalidInput(
                "this file would run, not open, in its default app: set an editor for it in \
                 Settings → Editors"
                    .into(),
            ));
        }
        self.opener().open(&file, app)
    }

    /// Show the review's file `path` from its local clone in the file
    /// manager.
    pub fn reveal_local_file(&self, id: ReviewId, path: &str) -> CoreResult<()> {
        let file = self.openable(id, path)?;
        self.opener().reveal(&file)
    }
}

#[cfg(test)]
mod tests;
