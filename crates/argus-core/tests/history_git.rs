//! Ground truth for the line walk: a real temporary git repository whose
//! commits move, rewrite, rename and delete lines; the walk runs over the
//! per-commit patches `git` generates (the same unified diffs the platforms
//! serve), and every answer is checked against `git blame` and
//! `git log -L`.
//!
//! The same corpus runs through both sources: the test's [`GitSource`]
//! (per-commit patches shaped as the platform APIs serve them, the API
//! path) and Argus' own local-clone source ([`LocalClone`]), which must
//! agree with git and with each other, and leave the clone untouched.
//! Skipped when `git` is not installed; Unix-only (paths, shell-free but
//! line-ending sensitive).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::SystemTime;

use argus_core::git::LocalClone;
use argus_core::history::{walk_line, LineHistorySource, LineStatus, WalkStart};
use argus_core::provider::{ChangedFile, FileDiff, FileStatus};
use argus_core::{CoreError, CoreResult};

struct Repo {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Repo {
    fn git(&self, args: &[&str]) -> String {
        let out = self.git_raw(args);
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn git_raw(&self, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .args(args)
            .current_dir(&self.path)
            // Isolated from the user's config: no signing, no hooks, no
            // pager, fixed identity and dates.
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture Author")
            .env("GIT_AUTHOR_EMAIL", "author@example.test")
            .env("GIT_COMMITTER_NAME", "Fixture Author")
            .env("GIT_COMMITTER_EMAIL", "author@example.test")
            .env("GIT_AUTHOR_DATE", "2026-10-10T10:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-10-10T10:00:00Z")
            .output()
            .expect("run git")
    }

    fn write(&self, path: &str, text: &str) {
        let p = self.path.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.path.join(path)).unwrap()
    }

    fn commit(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--no-verify", "-m", message]);
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }
}

/// [`LineHistorySource`] over a local clone, through plain `git`; commit
/// diffs memoised as the API source does.
struct GitSource<'a>(&'a Repo, Mutex<HashMap<String, FileDiff>>);

impl LineHistorySource for GitSource<'_> {
    fn commits_between(&self, from: &str, to: &str) -> CoreResult<Option<Vec<String>>> {
        let ancestor = self
            .0
            .git_raw(&["merge-base", "--is-ancestor", from, to])
            .status
            .success();
        if !ancestor {
            return Ok(None);
        }
        let list = self.0.git(&[
            "rev-list",
            "--reverse",
            "--first-parent",
            &format!("{from}..{to}"),
        ]);
        Ok(Some(list.lines().map(str::to_string).collect()))
    }

    fn commit_diff(&self, sha: &str) -> CoreResult<FileDiff> {
        if let Some(d) = self.1.lock().unwrap().get(sha) {
            return Ok(d.clone());
        }
        let parent = self.0.git(&["rev-parse", &format!("{sha}^")]);
        let parent = parent.trim();
        let mut diff = parse_git_diff(&self.0.git(&["diff", "-M", "--no-color", parent, sha]));
        diff.base_sha = parent.to_string();
        diff.head_sha = sha.to_string();
        self.1.lock().unwrap().insert(sha.to_string(), diff.clone());
        Ok(diff)
    }

    fn direct_diff(&self, from: &str, to: &str) -> CoreResult<Option<FileDiff>> {
        let mut diff = parse_git_diff(&self.0.git(&["diff", "-M", "--no-color", from, to]));
        diff.base_sha = from.to_string();
        diff.head_sha = to.to_string();
        Ok(Some(diff))
    }
}

/// `git diff` output as the platforms' file lists: one entry per file,
/// the patch being the hunks only (as GitHub's `patch` / GitLab's `diff`).
fn parse_git_diff(out: &str) -> FileDiff {
    let mut files: Vec<ChangedFile> = Vec::new();
    let mut sections: Vec<Vec<&str>> = Vec::new();
    for line in out.lines() {
        if line.starts_with("diff --git ") {
            sections.push(Vec::new());
        }
        if let Some(s) = sections.last_mut() {
            s.push(line);
        }
    }
    for s in sections {
        let header = s[0].strip_prefix("diff --git a/").unwrap();
        let (a, b) = header.split_once(" b/").unwrap();
        let hunk_start = s.iter().position(|l| l.starts_with("@@"));
        let meta = &s[..hunk_start.unwrap_or(s.len())];
        let has = |p: &str| meta.iter().any(|l| l.starts_with(p));
        let status = if has("deleted file mode") {
            FileStatus::Removed
        } else if has("new file mode") {
            FileStatus::Added
        } else if has("rename from") {
            FileStatus::Renamed
        } else {
            FileStatus::Modified
        };
        let patch = hunk_start.map(|i| s[i..].join("\n"));
        files.push(ChangedFile {
            path: if status == FileStatus::Removed { a } else { b }.to_string(),
            old_path: (status == FileStatus::Renamed).then(|| a.to_string()),
            status,
            additions: 0,
            deletions: 0,
            binary: has("Binary files"),
            too_large: false,
            patch,
            content_sha: String::new(),
        });
    }
    FileDiff {
        base_sha: String::new(),
        head_sha: String::new(),
        files,
        truncated: false,
    }
}

fn lines(range: std::ops::RangeInclusive<u32>) -> Vec<String> {
    range.map(|i| format!("line {i}")).collect()
}

fn text(lines: &[String]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

/// The history the walk is checked on (line numbers are the first
/// version's):
///
/// - c1: `src/lib.rs`, 30 lines (the comments are made here)
/// - c2: 3 lines in at the top; another file changed
/// - c3: line 10 rewritten; lines 20-21 deleted
/// - c4: renamed to `src/core.rs`, a line appended
/// - c5: 2 lines in after line 15; line 25 rewritten
/// - c6: lines 5 and 12 deleted
/// - c7: the file deleted
fn build() -> Option<(Repo, Vec<String>)> {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping the git ground-truth test: git is not installed");
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = Repo {
        path: dir.path().to_path_buf(),
        _dir: dir,
    };
    repo.git(&["init", "-q", "-b", "main"]);
    let mut v = lines(1..=30);
    repo.write("src/lib.rs", &text(&v));
    repo.write("README.md", "readme\n");
    let c1 = repo.commit("c1: add lib");

    for (i, h) in ["// header a", "// header b", "// header c"]
        .iter()
        .enumerate()
    {
        v.insert(i, h.to_string());
    }
    repo.write("src/lib.rs", &text(&v));
    repo.write("README.md", "readme, more\n");
    let c2 = repo.commit("c2: header");

    let at = |v: &Vec<String>, s: &str| v.iter().position(|l| l == s).unwrap();
    let i = at(&v, "line 10");
    v[i] = "line 10 changed".into();
    let i = at(&v, "line 20");
    v.drain(i..i + 2);
    repo.write("src/lib.rs", &text(&v));
    let c3 = repo.commit("c3: rewrite 10, drop 20-21");

    repo.git(&["mv", "src/lib.rs", "src/core.rs"]);
    v.push("// end".into());
    repo.write("src/core.rs", &text(&v));
    let c4 = repo.commit("c4: rename");

    let i = at(&v, "line 15");
    v.insert(i + 1, "inserted b".into());
    v.insert(i + 1, "inserted a".into());
    let i = at(&v, "line 25");
    v[i] = "line 25 changed".into();
    repo.write("src/core.rs", &text(&v));
    let c5 = repo.commit("c5: insert after 15, rewrite 25");

    let i = at(&v, "line 5");
    v.remove(i);
    let i = at(&v, "line 12");
    v.remove(i);
    repo.write("src/core.rs", &text(&v));
    assert_eq!(repo.read("src/core.rs"), text(&v));
    let c6 = repo.commit("c6: drop 5 and 12");

    repo.git(&["rm", "-q", "src/core.rs"]);
    let c7 = repo.commit("c7: remove");
    Some((repo, vec![c1, c2, c3, c4, c5, c6, c7]))
}

/// `git blame --porcelain` of one line: (origin commit, origin line,
/// origin file).
fn blame(repo: &Repo, rev: &str, path: &str, line: u32) -> (String, u32, String) {
    let out = repo.git(&[
        "-c",
        "core.quotePath=false",
        "blame",
        "--porcelain",
        "-L",
        &format!("{line},{line}"),
        rev,
        "--",
        path,
    ]);
    let mut first = out.lines().next().unwrap().split_whitespace();
    let sha = first.next().unwrap().to_string();
    let orig: u32 = first.next().unwrap().parse().unwrap();
    let file = out
        .lines()
        .find_map(|l| l.strip_prefix("filename "))
        .unwrap()
        .to_string();
    (sha, orig, file)
}

/// The commits `git log -L` names for one line at `rev`, oldest first.
fn log_l(repo: &Repo, rev: &str, path: &str, line: u32) -> Vec<String> {
    let out = repo.git(&[
        "log",
        "--format=COMMIT %H",
        "-L",
        &format!("{line},{line}:{path}"),
        rev,
    ]);
    let mut shas: Vec<String> = out
        .lines()
        .filter_map(|l| l.strip_prefix("COMMIT "))
        .map(str::to_string)
        .collect();
    shas.reverse();
    shas
}

#[test]
#[cfg_attr(windows, ignore)]
fn the_walk_agrees_with_git() {
    let Some((repo, commits)) = build() else {
        return;
    };
    check_against_git(
        &repo,
        &commits,
        &GitSource(&repo, Mutex::new(HashMap::new())),
    );
}

#[test]
#[cfg_attr(windows, ignore)]
fn the_clone_walk_agrees_with_git_and_with_the_api_walk() {
    let Some((repo, commits)) = build() else {
        return;
    };
    let clone = LocalClone::open(&repo.path).expect("a usable clone");
    check_against_git(&repo, &commits, &clone);

    // Line by line, range by range, at every head: the same answer, events
    // and all.
    let api = GitSource(&repo, Mutex::new(HashMap::new()));
    let mut compared = 0;
    for head in &commits[1..] {
        for (start_line, line) in (1..=30).map(|l| (l, l)).chain([(8, 12), (18, 23), (4, 6)]) {
            let start = WalkStart {
                sha: commits[0].clone(),
                path: "src/lib.rs".into(),
                start_line,
                line,
            };
            assert_eq!(
                walk_line(&clone, &start, head).unwrap(),
                walk_line(&api, &start, head).unwrap(),
                "{start_line}-{line} at {head}"
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 6 * 33);

    // From a later commit too.
    let start = WalkStart {
        sha: commits[2].clone(),
        path: "src/lib.rs".into(),
        start_line: 11,
        line: 11,
    };
    assert_eq!(
        walk_line(&clone, &start, &commits[5]).unwrap(),
        walk_line(&api, &start, &commits[5]).unwrap()
    );

    // A comment on a commit force-pushed away (still in the clone): no line
    // of commits to head, so one direct diff, exactly from that commit.
    repo.git(&["checkout", "-q", "-b", "old", &commits[0]]);
    let mut v = lines(1..=30);
    v.insert(0, "// gone".into());
    repo.write("src/lib.rs", &text(&v));
    let old = repo.commit("old: a line on top");
    repo.git(&["checkout", "-q", "main"]);
    assert_eq!(clone.commits_between(&old, &commits[5]).unwrap(), None);
    for line in [1, 8, 11, 26] {
        let start = WalkStart {
            sha: old.clone(),
            path: "src/lib.rs".into(),
            start_line: line,
            line,
        };
        let m = walk_line(&clone, &start, &commits[5]).unwrap();
        assert!(!m.attributed, "line {line}");
        assert_eq!(
            m,
            walk_line(&api, &start, &commits[5]).unwrap(),
            "line {line}"
        );
    }
    // Old line 8 is c1's line 7 under one extra line: both land on the same
    // line at head.
    let at_head = |sha: &str, line| {
        walk_line(
            &clone,
            &WalkStart {
                sha: sha.to_string(),
                path: "src/lib.rs".into(),
                start_line: line,
                line,
            },
            &commits[5],
        )
        .unwrap()
        .status
    };
    let (LineStatus::Located { line: a, .. }, LineStatus::Located { line: b, .. }) =
        (at_head(&old, 8), at_head(&commits[0], 7))
    else {
        panic!("not located");
    };
    assert_eq!(a, b);
}

/// What a read could have changed in a clone: worktree and index state
/// (through a status that takes no lock itself), HEAD, refs, and every
/// file under `.git` with its size and modification time.
fn clone_state(repo: &Repo) -> (String, String, String, Vec<(String, u64, SystemTime)>) {
    let out = Command::new("git")
        .args(["status", "--porcelain=v1", "--ignored", "-uall"])
        .current_dir(&repo.path)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success());
    let status = String::from_utf8(out.stdout).unwrap();
    let head = repo.git(&["rev-parse", "HEAD"]) + &repo.git(&["symbolic-ref", "HEAD"]);
    let refs = repo.git(&["for-each-ref"]);
    let mut files = Vec::new();
    let mut dirs = vec![repo.path.join(".git")];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let meta = e.metadata().unwrap();
            if meta.is_dir() {
                dirs.push(e.path());
            } else {
                let rel = e
                    .path()
                    .strip_prefix(&repo.path)
                    .unwrap()
                    .display()
                    .to_string();
                files.push((rel, meta.len(), meta.modified().unwrap()));
            }
        }
    }
    files.sort();
    (status, head, refs, files)
}

#[test]
#[cfg_attr(windows, ignore)]
fn the_clone_is_read_exactly_and_never_changed() {
    let Some((repo, commits)) = build() else {
        return;
    };
    // The user's own work in progress: a modified tracked file, a staged
    // file, an untracked one.
    repo.write("README.md", "local edit\n");
    repo.write("staged.txt", "staged\n");
    repo.git(&["add", "staged.txt"]);
    repo.write("scratch.txt", "untracked\n");
    let before = clone_state(&repo);
    assert!(before.0.contains(" M README.md") && before.0.contains("?? scratch.txt"));

    let clone = LocalClone::open(&repo.path).unwrap();
    // Files: exactly the committed blobs, whatever the working tree says.
    let mut read = 0;
    for sha in &commits {
        let paths = repo.git(&["ls-tree", "-r", "--name-only", sha]);
        for path in paths.lines() {
            let want = repo.git(&["show", &format!("{sha}:{path}")]);
            assert_eq!(
                clone.file_at(sha, path).unwrap().unwrap(),
                want,
                "{path}@{sha}"
            );
            read += 1;
        }
        assert!(matches!(
            clone.file_at(sha, "staged.txt"),
            Some(Err(CoreError::NotFound))
        ));
    }
    assert!(read > 10);
    let head = commits.last().unwrap();
    assert_eq!(
        clone.file_at(head, "README.md").unwrap().unwrap(),
        "readme, more\n"
    );
    // Walks, commit diffs and direct diffs over the whole history.
    check_against_git(&repo, &commits, &clone);
    for sha in &commits[1..] {
        clone.commit_diff(sha).unwrap();
    }
    clone.direct_diff(&commits[0], head).unwrap().unwrap();
    // A commit the clone has never seen: not there, and not fetched.
    let unknown = "ab".repeat(20);
    assert!(!clone.has_commit(&unknown));
    assert!(clone.file_at(&unknown, "README.md").is_none());

    assert_eq!(clone_state(&repo), before);
}

#[test]
#[cfg_attr(windows, ignore)]
fn paths_with_spaces_and_non_ascii_are_followed() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = Repo {
        path: dir.path().to_path_buf(),
        _dir: dir,
    };
    repo.git(&["init", "-q", "-b", "main"]);
    let (from, to) = ("src/my file é.rs", "src/über dir/b \"q\" file.rs");
    let mut v = lines(1..=30);
    repo.write(from, &text(&v));
    let c1 = repo.commit("c1");
    repo.git(&["mv", from, "src/tmp.rs"]);
    std::fs::create_dir_all(repo.path.join("src/über dir")).unwrap();
    repo.git(&["mv", "src/tmp.rs", to]);
    v.insert(0, "// top".into());
    v[5] = "line 5 changed".into();
    repo.write(to, &text(&v));
    let c2 = repo.commit("c2: rename and edit");
    v[20] = "line 20 changed".into();
    repo.write(to, &text(&v));
    let c3 = repo.commit("c3: edit");

    let clone = LocalClone::open(&repo.path).unwrap();
    for line in 1..=30 {
        let m = walk_line(
            &clone,
            &WalkStart {
                sha: c1.clone(),
                path: from.into(),
                start_line: line,
                line,
            },
            &c3,
        )
        .unwrap();
        let LineStatus::Located {
            path,
            line: now,
            renamed,
            ..
        } = &m.status
        else {
            panic!("line {line}: {:?}", m.status);
        };
        assert_eq!((path.as_str(), *renamed), (to, true));
        let (origin, origin_line, origin_file) = blame(&repo, &c3, path, *now);
        match m.touched_by.last() {
            None => assert_eq!(
                (origin.as_str(), origin_line, origin_file.as_str()),
                (c1.as_str(), line, from),
                "line {line}"
            ),
            Some(by) => assert_eq!(&origin, by, "line {line}"),
        }
    }
    assert_eq!(
        clone.file_at(&c2, to).unwrap().unwrap(),
        repo.git(&["show", &format!("{c2}:{to}")])
    );
}

/// Every line of c1 walked to every later head through `src`, checked
/// against `git blame` / `git log -L`, plus the story of a few lines.
fn check_against_git(repo: &Repo, commits: &[String], src: &dyn LineHistorySource) {
    let c = |i: usize| commits[i - 1].clone();
    let walk = |line: u32, head: &str| {
        walk_line(
            src,
            &WalkStart {
                sha: c(1),
                path: "src/lib.rs".into(),
                start_line: line,
                line,
            },
            head,
        )
        .unwrap()
    };

    let mut checked = 0;
    for head_index in 2..=7 {
        let head = c(head_index);
        for line in 1..=30 {
            let m = walk(line, &head);
            match &m.status {
                LineStatus::Located {
                    path,
                    line: now,
                    touched,
                    ..
                } => {
                    let (origin, origin_line, origin_file) = blame(repo, &head, path, *now);
                    if m.touched_by.is_empty() {
                        assert!(!touched);
                        // Untouched: git traces the line at head back to
                        // exactly the commented line.
                        assert_eq!(
                            (origin.as_str(), origin_line, origin_file.as_str()),
                            (c(1).as_str(), line, "src/lib.rs"),
                            "line {line} at c{head_index}"
                        );
                    } else {
                        assert_eq!(&origin, m.touched_by.last().unwrap(), "line {line}");
                    }
                    // The commits git names for the line since c1 are the
                    // ones the walk says touched it.
                    let since: Vec<String> = log_l(repo, &head, path, *now)
                        .into_iter()
                        .filter(|s| *s != c(1))
                        .collect();
                    assert_eq!(since, m.touched_by, "line {line} at c{head_index}");
                }
                LineStatus::Deleted {
                    removed_by: Some(by),
                    ..
                } => {
                    // Right before that commit the line was at a place
                    // git's reverse blame sees end in exactly that commit.
                    let parent = repo.git(&["rev-parse", &format!("{by}^")]);
                    let parent = parent.trim();
                    let before = walk(line, parent);
                    let LineStatus::Located {
                        path, line: then, ..
                    } = &before.status
                    else {
                        panic!("line {line} not located before {by}");
                    };
                    let out = repo.git(&[
                        "blame",
                        "--porcelain",
                        "--reverse",
                        "-L",
                        &format!("{then},{then}"),
                        &format!("{parent}..{by}"),
                        "--",
                        path,
                    ]);
                    let last_seen = out.split_whitespace().next().unwrap();
                    assert_eq!(last_seen, parent, "line {line} deleted in {by}");
                    assert_eq!(m.touched_by.last(), Some(by));
                }
                LineStatus::FileGone {
                    path,
                    removed_by: Some(by),
                } => {
                    let parent = format!("{by}^");
                    assert!(repo
                        .git_raw(&["cat-file", "-e", &format!("{parent}:{path}")])
                        .status
                        .success());
                    assert!(!repo
                        .git_raw(&["cat-file", "-e", &format!("{by}:{path}")])
                        .status
                        .success());
                }
                other => panic!("line {line} at c{head_index}: {other:?}"),
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 6 * 30);

    // The story, spelled out for a few lines.
    let at6 = |line| walk(line, &c(6));
    let LineStatus::Located {
        path,
        line,
        renamed,
        touched,
        ..
    } = at6(10).status
    else {
        panic!()
    };
    assert_eq!(
        (path.as_str(), line, renamed, touched),
        ("src/core.rs", 12, true, true)
    );
    assert_eq!(at6(10).touched_by, [c(3)]);
    assert_eq!(
        at6(20).status,
        // Deleted in c3, before the rename; the path is where its place is
        // at head.
        LineStatus::Deleted {
            path: "src/core.rs".into(),
            removed_by: Some(c(3))
        }
    );
    assert_eq!(
        at6(5).status,
        LineStatus::Deleted {
            path: "src/core.rs".into(),
            removed_by: Some(c(6))
        }
    );
    assert_eq!(at6(25).touched_by, [c(5)]);
    assert_eq!(
        walk(16, &c(7)).status,
        LineStatus::FileGone {
            path: "src/core.rs".into(),
            removed_by: Some(c(7))
        }
    );
    // Deleted before the file went: still "deleted" by its own commit.
    assert_eq!(
        walk(12, &c(7)).status,
        LineStatus::Deleted {
            path: "src/core.rs".into(),
            removed_by: Some(c(6))
        }
    );

    // One direct diff from c1 to c6 lands every surviving line in the
    // same place (it just cannot name the commits).
    let direct_diff = src.direct_diff(&c(1), &c(6)).unwrap().unwrap();
    for line in 1..=30 {
        let stepwise = walk(line, &c(6));
        let direct = {
            let diff = direct_diff.clone();
            struct Direct(FileDiff);
            impl LineHistorySource for Direct {
                fn commits_between(&self, _: &str, _: &str) -> CoreResult<Option<Vec<String>>> {
                    Ok(None)
                }
                fn commit_diff(&self, _: &str) -> CoreResult<FileDiff> {
                    unreachable!()
                }
                fn direct_diff(&self, _: &str, _: &str) -> CoreResult<Option<FileDiff>> {
                    Ok(Some(self.0.clone()))
                }
            }
            walk_line(
                &Direct(diff),
                &WalkStart {
                    sha: c(1),
                    path: "src/lib.rs".into(),
                    start_line: line,
                    line,
                },
                &c(6),
            )
            .unwrap()
        };
        match (&stepwise.status, &direct.status) {
            (
                LineStatus::Located {
                    line: a, path: p, ..
                },
                LineStatus::Located {
                    line: b, path: q, ..
                },
            ) => {
                assert_eq!((a, p), (b, q), "line {line}")
            }
            (LineStatus::Deleted { .. }, LineStatus::Deleted { .. }) => {}
            (a, b) => panic!("line {line}: stepwise {a:?}, direct {b:?}"),
        }
    }
}
