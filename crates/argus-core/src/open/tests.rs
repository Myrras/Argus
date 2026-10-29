use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use super::*;
use crate::provider::{
    DiffRange, FileDiff, ProviderId, ReviewDetail, ReviewProvider, ReviewRef, Thread,
};
use crate::store::Store;
use crate::testutil::{detail, ME};

// ------------------------------------------------------------ pure parts

fn canonical_tempdir() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    (dir, root)
}

#[test]
fn paths_that_leave_the_clone_are_refused() {
    let (_dir, root) = canonical_tempdir();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.rs"), "a").unwrap();
    for bad in [
        "",
        "/etc/passwd",
        "../outside.txt",
        "src/../../outside.txt",
        "src/../a.rs",
        "./src/a.rs",
        "src\\a.rs",
        "src/a\0.rs",
    ] {
        assert!(
            matches!(resolve(&root, bad), Err(CoreError::InvalidInput(_))),
            "{bad:?}"
        );
    }
    let ok = resolve(&root, "src/a.rs").unwrap();
    assert_eq!(ok.path, root.join("src/a.rs"));
    assert!(ok.is_file);
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_symlink_out_of_the_clone_is_refused_and_one_inside_followed() {
    let (_outside_dir, outside) = canonical_tempdir();
    std::fs::write(outside.join("secret.txt"), "s").unwrap();
    let (_dir, root) = canonical_tempdir();
    std::fs::write(root.join("real.txt"), "r").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(outside.join("secret.txt"), root.join("file-link")).unwrap();
        symlink(&outside, root.join("dir-link")).unwrap();
        symlink(root.join("real.txt"), root.join("inner-link")).unwrap();
    }
    for bad in ["file-link", "dir-link/secret.txt"] {
        assert!(
            matches!(resolve(&root, bad), Err(CoreError::InvalidInput(_))),
            "{bad}"
        );
    }
    let inner = resolve(&root, "inner-link").unwrap();
    assert_eq!(inner.path, root.join("real.txt"));
    assert!(inner.is_file);
    // Not in the outside folder either way: a missing name under the link
    // is no file, and nothing would open it.
    assert!(!resolve(&root, "dir-link/missing.txt").unwrap().is_file);
}

#[test]
fn a_missing_path_or_a_folder_is_no_file() {
    let (_dir, root) = canonical_tempdir();
    std::fs::create_dir(root.join("dir")).unwrap();
    let missing = resolve(&root, "gone.rs").unwrap();
    assert_eq!(missing.path, root.join("gone.rs"));
    assert!(!missing.is_file);
    assert!(!resolve(&root, "dir").unwrap().is_file);
}

#[test]
fn extensions_are_lower_case_without_the_dot() {
    assert_eq!(extension(Path::new("/c/a/Notes.MD")), Some("md".into()));
    assert_eq!(extension(Path::new("/c/a/x.tar.gz")), Some("gz".into()));
    assert_eq!(extension(Path::new("/c/a/.env")), None);
    assert_eq!(extension(Path::new("/c/a/Makefile")), None);
    assert_eq!(extension(Path::new("/c/a/trailing.")), None);
}

#[test]
fn the_editor_is_the_extension_override_then_the_default_then_the_os() {
    let mut s = Settings::default();
    assert_eq!(editor_for(&s, Path::new("/c/a.rs")), None);
    s.default_editor = Some("/Applications/Zed.app".into());
    s.editor_overrides
        .insert("md".into(), "/Applications/Typora.app".into());
    assert_eq!(
        editor_for(&s, Path::new("/c/README.MD")),
        Some("/Applications/Typora.app")
    );
    assert_eq!(
        editor_for(&s, Path::new("/c/a.rs")),
        Some("/Applications/Zed.app")
    );
    assert_eq!(
        editor_for(&s, Path::new("/c/Makefile")),
        Some("/Applications/Zed.app")
    );
    s.default_editor = None;
    assert_eq!(editor_for(&s, Path::new("/c/a.rs")), None);
}

#[test]
fn files_the_os_would_run_are_told_apart() {
    let (_dir, root) = canonical_tempdir();
    for name in ["a.rs", "deploy.command", "Setup.EXE", "x.app"] {
        std::fs::write(root.join(name), "x").unwrap();
    }
    assert!(!runs_on_open(&root.join("a.rs")));
    assert!(runs_on_open(&root.join("deploy.command")));
    assert!(runs_on_open(&root.join("Setup.EXE")));
    assert!(runs_on_open(&root.join("x.app")));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = root.join("build");
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        assert!(!runs_on_open(&script));
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(runs_on_open(&script));
    }
}

// ------------------------------------------------------------ the core

/// Records what it was asked to open or show; never launches anything.
#[derive(Default)]
struct FakeOpener {
    opened: Mutex<Vec<(PathBuf, Option<String>)>>,
    revealed: Mutex<Vec<PathBuf>>,
}

impl Opener for FakeOpener {
    fn open(&self, path: &Path, app: Option<&str>) -> CoreResult<()> {
        self.opened
            .lock()
            .unwrap()
            .push((path.to_path_buf(), app.map(str::to_string)));
        Ok(())
    }
    fn reveal(&self, path: &Path) -> CoreResult<()> {
        self.revealed.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }
}

struct Fake {
    detail: ReviewDetail,
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
        Ok(vec![self.detail.review.clone()])
    }
    fn fetch_review(&self, _: &ReviewRef) -> CoreResult<ReviewDetail> {
        Ok(self.detail.clone())
    }
    fn fetch_threads(&self, _: &ReviewRef) -> CoreResult<Vec<Thread>> {
        Ok(Vec::new())
    }
    fn file_at(&self, _: &ReviewRef, _: &str, _: &str) -> CoreResult<String> {
        Err(CoreError::NotFound)
    }
    fn fetch_files(&self, _: &ReviewRef, _: &DiffRange) -> CoreResult<FileDiff> {
        Err(CoreError::NotFound)
    }
}

/// A core with one review at `head`, `clone` as its repository's clone (if
/// any), and a fake opener.
fn core_at(head: &str, clone: Option<&Path>) -> (Core, ReviewId, Arc<FakeOpener>) {
    let fake = Arc::new(Fake {
        detail: detail(head),
    });
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![fake]);
    assert!(core.refresh().errors.is_empty());
    let id = core.reviews().unwrap()[0].id;
    if let Some(dir) = clone {
        let mut s = core.settings();
        s.clone_paths
            .insert("github.com/acme-fixture/widgets".into(), dir.to_path_buf());
        core.update_settings(s).unwrap();
    }
    let opener = Arc::new(FakeOpener::default());
    core.set_opener(opener.clone());
    (core, id, opener)
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture Author")
        .env("GIT_AUTHOR_EMAIL", "author@example.test")
        .env("GIT_COMMITTER_NAME", "Fixture Author")
        .env("GIT_COMMITTER_EMAIL", "author@example.test")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A clone with `src/cache.rs` and `README.md` committed; its head.
fn repo() -> Option<(tempfile::TempDir, PathBuf, String)> {
    let (dir, root) = canonical_tempdir();
    git(&root, &["init", "-q", "-b", "main"])?;
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(root.join("src/cache.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("README.md"), "# widgets\n").unwrap();
    git(&root, &["add", "-A"])?;
    git(&root, &["commit", "-q", "--no-verify", "-m", "c1"])?;
    let head = git(&root, &["rev-parse", "HEAD"])?;
    Some((dir, root, head))
}

#[test]
fn without_a_clone_there_is_nothing_to_open() {
    let (core, id, opener) = core_at("h1", None);
    assert_eq!(
        core.local_files(id, &["src/cache.rs".into()]).unwrap(),
        None
    );
    assert_eq!(core.local_file(id, "src/cache.rs").unwrap(), None);
    assert!(matches!(
        core.open_in_editor(id, "src/cache.rs", false),
        Err(CoreError::InvalidInput(_))
    ));
    // A clone set to a folder that is gone counts as none.
    let (core, id, _) = core_at("h1", Some(Path::new("/nonexistent/argus-fixture")));
    assert_eq!(core.local_file(id, "src/cache.rs").unwrap(), None);
    assert!(opener.opened.lock().unwrap().is_empty());
}

#[test]
#[cfg_attr(windows, ignore)]
fn files_in_the_clone_say_whether_they_exist_and_the_head_matches() {
    let Some((_dir, root, head)) = repo() else {
        eprintln!("skipping: git is not installed");
        return;
    };
    let (core, id, _) = core_at(&head, Some(&root));
    let files = core
        .local_files(
            id,
            &[
                "src/cache.rs".into(),
                "src/removed.rs".into(),
                "../escape.rs".into(),
            ],
        )
        .unwrap()
        .unwrap();
    // The escaping path is left out.
    assert_eq!(files.len(), 2);
    assert_eq!(
        files[0],
        LocalFile {
            path: "src/cache.rs".into(),
            absolute_path: root.join("src/cache.rs"),
            exists: true,
            clone_head: Some(head.clone()),
            matches_review_head: true,
        }
    );
    assert!(!files[1].exists);
    assert!(matches!(
        core.local_file(id, "../escape.rs"),
        Err(CoreError::InvalidInput(_))
    ));

    // The clone moves on: the file is still there, the head no longer
    // matches the review's.
    std::fs::write(root.join("src/cache.rs"), "fn main() { 1; }\n").unwrap();
    git(&root, &["commit", "-q", "-am", "c2"]).unwrap();
    let moved = core.local_file(id, "src/cache.rs").unwrap().unwrap();
    assert!(moved.exists);
    assert!(!moved.matches_review_head);
    assert_ne!(moved.clone_head.as_deref(), Some(head.as_str()));
    assert_eq!(moved.clone_head.unwrap().len(), 40);
}

#[test]
fn a_clone_that_is_no_repository_still_opens_its_files() {
    // git is not needed to open a file: only the head is unknown.
    let (_dir, root) = canonical_tempdir();
    std::fs::write(root.join("a.rs"), "a").unwrap();
    let (core, id, opener) = core_at("h1", Some(&root));
    let f = core.local_file(id, "a.rs").unwrap().unwrap();
    assert!(f.exists);
    assert_eq!(f.clone_head, None);
    assert!(!f.matches_review_head);
    core.open_in_editor(id, "a.rs", false).unwrap();
    assert_eq!(opener.opened.lock().unwrap().len(), 1);
}

#[test]
fn opening_uses_the_editor_set_for_the_file() {
    let (_dir, root) = canonical_tempdir();
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(root.join("src/cache.rs"), "x").unwrap();
    std::fs::write(root.join("README.md"), "x").unwrap();
    let (core, id, opener) = core_at("h1", Some(&root));
    let mut s = core.settings();
    s.editor_overrides
        .insert("md".into(), "/Applications/Typora.app".into());
    core.update_settings(s.clone()).unwrap();

    core.open_in_editor(id, "README.md", false).unwrap();
    core.open_in_editor(id, "src/cache.rs", false).unwrap();
    s.default_editor = Some("/Applications/Zed.app".into());
    core.update_settings(s).unwrap();
    core.open_in_editor(id, "src/cache.rs", false).unwrap();
    // "Open with default app" skips the settings.
    core.open_in_editor(id, "README.md", true).unwrap();
    core.reveal_local_file(id, "src/cache.rs").unwrap();

    let opened = opener.opened.lock().unwrap().clone();
    let typora = Some("/Applications/Typora.app".to_string());
    let zed = Some("/Applications/Zed.app".to_string());
    assert_eq!(
        opened,
        [
            (root.join("README.md"), typora),
            (root.join("src/cache.rs"), None),
            (root.join("src/cache.rs"), zed),
            (root.join("README.md"), None),
        ]
    );
    assert_eq!(
        *opener.revealed.lock().unwrap(),
        [root.join("src/cache.rs")]
    );
}

#[test]
fn removed_escaping_or_runnable_files_are_never_handed_over() {
    let (_dir, root) = canonical_tempdir();
    std::fs::write(root.join("deploy.command"), "rm -rf /\n").unwrap();
    let (core, id, opener) = core_at("h1", Some(&root));
    for (path, use_default) in [
        ("gone.rs", false),
        ("../outside.rs", false),
        ("/etc/hosts", false),
        // Its default app would run it.
        ("deploy.command", false),
        ("deploy.command", true),
    ] {
        assert!(
            matches!(
                core.open_in_editor(id, path, use_default),
                Err(CoreError::InvalidInput(_))
            ),
            "{path}"
        );
    }
    assert!(matches!(
        core.reveal_local_file(id, "gone.rs"),
        Err(CoreError::InvalidInput(_))
    ));
    assert!(opener.opened.lock().unwrap().is_empty());
    assert!(opener.revealed.lock().unwrap().is_empty());

    // With an editor chosen for it, it opens in that editor.
    let mut s = core.settings();
    s.editor_overrides
        .insert("command".into(), "/Applications/Zed.app".into());
    core.update_settings(s).unwrap();
    core.open_in_editor(id, "deploy.command", false).unwrap();
    assert_eq!(opener.opened.lock().unwrap().len(), 1);
}

#[test]
fn a_core_without_an_opener_says_so() {
    let (_dir, root) = canonical_tempdir();
    std::fs::write(root.join("a.rs"), "a").unwrap();
    let fake = Arc::new(Fake {
        detail: detail("h1"),
    });
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![fake]);
    core.refresh();
    let id = core.reviews().unwrap()[0].id;
    let mut s = core.settings();
    s.clone_paths
        .insert("github.com/acme-fixture/widgets".into(), root.clone());
    core.update_settings(s).unwrap();
    let err = core.open_in_editor(id, "a.rs", false).unwrap_err();
    assert!(err.to_string().contains("not set up"), "{err}");
}

// ----------------------------------------------------------- clone status

/// Every file under `dir` with its size and mtime: a read must change none.
fn fingerprint(dir: &Path) -> Vec<(PathBuf, u64, Option<std::time::SystemTime>)> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                todo.push(path);
            } else {
                out.push((path, meta.len(), meta.modified().ok()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn no_clone_set_is_not_configured() {
    let (core, id, _) = core_at("h1", None);
    assert_eq!(core.clone_status(id).unwrap(), CloneStatus::default());
    assert!(!core.clone_status(id).unwrap().configured);
}

#[test]
fn a_clone_folder_that_is_gone_is_configured_but_not_found() {
    let gone = Path::new("/nonexistent/argus-fixture/widgets");
    let (core, id, _) = core_at("h1", Some(gone));
    let status = core.clone_status(id).unwrap();
    assert!(status.configured);
    assert_eq!(status.path.as_deref(), Some(gone));
    assert!(!status.found);
    assert_eq!(status.head, None);
    assert!(!status.matches_review_head);
}

#[test]
fn a_folder_that_is_no_repository_is_configured_but_not_found() {
    let (_dir, root) = canonical_tempdir();
    std::fs::write(root.join("a.rs"), "a").unwrap();
    let (core, id, _) = core_at("h1", Some(&root));
    let status = core.clone_status(id).unwrap();
    assert!(status.configured);
    assert_eq!(status.path.as_deref(), Some(root.as_path()));
    assert!(!status.found);
    assert_eq!(status.head, None);
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_repository_is_found_with_its_head_and_whether_it_matches() {
    let Some((_dir, root, head)) = repo() else {
        eprintln!("skipping: git is not installed");
        return;
    };
    let (core, id, _) = core_at(&head, Some(&root));
    let status = core.clone_status(id).unwrap();
    assert_eq!(
        status,
        CloneStatus {
            configured: true,
            path: Some(root.clone()),
            found: true,
            head: Some(head.clone()),
            matches_review_head: true,
        }
    );

    // The clone moves on: still found, no longer at the review's head.
    std::fs::write(root.join("README.md"), "# widgets 2\n").unwrap();
    git(&root, &["commit", "-q", "-am", "c2"]).unwrap();
    let moved = core.clone_status(id).unwrap();
    assert!(moved.found);
    assert!(!moved.matches_review_head);
    assert_ne!(moved.head.as_deref(), Some(head.as_str()));
}

#[test]
#[cfg_attr(windows, ignore)]
fn reading_the_status_changes_nothing_in_the_clone() {
    let Some((_dir, root, head)) = repo() else {
        eprintln!("skipping: git is not installed");
        return;
    };
    let (core, id, opener) = core_at(&head, Some(&root));
    let before = fingerprint(&root);
    core.clone_status(id).unwrap();
    core.clone_status(id).unwrap();
    assert_eq!(fingerprint(&root), before);
    // Reading never launches anything either.
    assert!(opener.opened.lock().unwrap().is_empty());
    assert!(opener.revealed.lock().unwrap().is_empty());
}

#[test]
fn revealing_the_clone_shows_its_folder_only() {
    let (_dir, root) = canonical_tempdir();
    let (core, id, opener) = core_at("h1", Some(&root));
    core.reveal_clone(id).unwrap();
    assert_eq!(
        *opener.revealed.lock().unwrap(),
        std::slice::from_ref(&root)
    );
    assert!(opener.opened.lock().unwrap().is_empty());
}

#[test]
fn there_is_no_clone_to_reveal_without_a_folder() {
    let (core, id, opener) = core_at("h1", None);
    assert!(matches!(
        core.reveal_clone(id),
        Err(CoreError::InvalidInput(_))
    ));
    let (core, id, _) = core_at("h1", Some(Path::new("/nonexistent/argus-fixture")));
    assert!(matches!(
        core.reveal_clone(id),
        Err(CoreError::InvalidInput(_))
    ));
    assert!(opener.revealed.lock().unwrap().is_empty());
}

#[test]
fn a_file_set_as_the_clone_is_not_revealed() {
    let (_dir, root) = canonical_tempdir();
    std::fs::write(root.join("a.rs"), "a").unwrap();
    let (core, id, opener) = core_at("h1", Some(&root.join("a.rs")));
    assert!(matches!(
        core.reveal_clone(id),
        Err(CoreError::InvalidInput(_))
    ));
    assert!(opener.revealed.lock().unwrap().is_empty());
}

/// Records project opens apart from file opens.
#[derive(Default)]
struct ProjectOpener {
    projects: Mutex<Vec<(String, PathBuf, PathBuf)>>,
    files: Mutex<Vec<(PathBuf, Option<String>)>>,
}

impl Opener for ProjectOpener {
    fn open(&self, path: &Path, app: Option<&str>) -> CoreResult<()> {
        self.files
            .lock()
            .unwrap()
            .push((path.to_path_buf(), app.map(str::to_string)));
        Ok(())
    }
    fn reveal(&self, _path: &Path) -> CoreResult<()> {
        Ok(())
    }
    fn open_project_file(&self, app: &str, root: &Path, file: &Path) -> CoreResult<()> {
        self.projects.lock().unwrap().push((
            app.to_string(),
            root.to_path_buf(),
            file.to_path_buf(),
        ));
        Ok(())
    }
}

#[test]
fn an_editor_gets_the_repository_with_the_file() {
    let (_dir, root) = canonical_tempdir();
    std::fs::write(root.join("a.rs"), "x").unwrap();
    std::fs::write(root.join("b.md"), "x").unwrap();
    let (core, id, _) = core_at("h1", Some(&root));
    let opener = Arc::new(ProjectOpener::default());
    core.set_opener(opener.clone());
    let mut s = core.settings();
    assert!(s.open_with_repository, "on by default");
    s.default_editor = Some("/Applications/Zed.app".into());
    core.update_settings(s.clone()).unwrap();

    core.open_in_editor(id, "a.rs", false).unwrap();
    assert_eq!(
        *opener.projects.lock().unwrap(),
        [(
            "/Applications/Zed.app".to_string(),
            root.clone(),
            root.join("a.rs")
        )]
    );
    // The OS default app: the file only.
    core.open_in_editor(id, "b.md", true).unwrap();
    assert_eq!(*opener.files.lock().unwrap(), [(root.join("b.md"), None)]);
    // Switch off: today's behaviour.
    s.open_with_repository = false;
    core.update_settings(s).unwrap();
    core.open_in_editor(id, "a.rs", false).unwrap();
    assert_eq!(opener.projects.lock().unwrap().len(), 1);
    assert_eq!(
        opener.files.lock().unwrap()[1],
        (root.join("a.rs"), Some("/Applications/Zed.app".to_string()))
    );
}

#[test]
fn project_open_falls_back_to_the_file() {
    let (_dir, root) = canonical_tempdir();
    std::fs::write(root.join("a.rs"), "x").unwrap();
    let (core, id, opener) = core_at("h1", Some(&root));
    let mut s = core.settings();
    s.default_editor = Some("/Applications/Zed.app".into());
    core.update_settings(s).unwrap();
    core.open_in_editor(id, "a.rs", false).unwrap();
    assert_eq!(
        *opener.opened.lock().unwrap(),
        [(root.join("a.rs"), Some("/Applications/Zed.app".to_string()))]
    );
}

#[test]
fn an_old_settings_file_opens_the_repository() {
    let old: Settings = serde_json::from_str(r#"{"pollIntervalSecs": 60}"#).unwrap();
    assert!(old.open_with_repository);
}
