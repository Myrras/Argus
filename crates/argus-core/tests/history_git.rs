//! Ground truth for the line walk: a real temporary git repository whose
//! commits move, rewrite, rename and delete lines; the walk runs over the
//! per-commit patches `git` generates (the same unified diffs the platforms
//! serve), and every answer is checked against `git blame` and
//! `git log -L`.
//!
//! The [`GitSource`] here is also a sketch of the M6 local-clone source.
//! Skipped when `git` is not installed; Unix-only (paths, shell-free but
//! line-ending sensitive).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

use argus_core::history::{walk_line, LineHistorySource, LineStatus, WalkStart};
use argus_core::provider::{ChangedFile, FileDiff, FileStatus};
use argus_core::CoreResult;

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
    let src = GitSource(&repo, Mutex::new(HashMap::new()));
    let c = |i: usize| commits[i - 1].clone();
    let walk = |line: u32, head: &str| {
        walk_line(
            &src,
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
                    let (origin, origin_line, origin_file) = blame(&repo, &head, path, *now);
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
                    let since: Vec<String> = log_l(&repo, &head, path, *now)
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
