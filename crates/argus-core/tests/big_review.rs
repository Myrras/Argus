//! A huge merge request through the GitLab double and the fake glab: the
//! file list, the files' contents and the commentable lines of 400 files.
//! `big_review_timings` is a measurement, not a check (`--ignored
//! --nocapture` prints where the time goes); the other tests pin the
//! behaviour the big-review work relies on.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use argus_core::store::Store;
use argus_core::{Core, ReviewId};
use serde_json::json;

use support::gitlab::*;

pub const FILES: usize = 400;

fn path_of(i: usize) -> String {
    format!("src/module{}/file{i}.rs", i % 25)
}

/// The file's text at the base or the head: 600 lines, a hunk every 30.
pub fn text(i: usize, head: bool) -> String {
    (0..600)
        .map(|n| {
            if head && n % 30 == 10 {
                format!("    let changed_{i}_{n} = compute({n});\n")
            } else {
                format!("    let value_{i}_{n} = compute({n});\n")
            }
        })
        .collect()
}

fn patch(i: usize) -> String {
    let mut out = String::new();
    for n in (10..600).step_by(30) {
        out.push_str(&format!(
            "@@ -{a},7 +{a},7 @@\n     let value_{i}_{x} = compute({x});\n     let value_{i}_{y} = compute({y});\n     let value_{i}_{z} = compute({z});\n-    let value_{i}_{n} = compute({n});\n+    let changed_{i}_{n} = compute({n});\n     let value_{i}_{p} = compute({p});\n     let value_{i}_{q} = compute({q});\n     let value_{i}_{r} = compute({r});\n",
            a = n - 2,
            x = n - 3,
            y = n - 2,
            z = n - 1,
            p = n + 1,
            q = n + 2,
            r = n + 3,
        ));
    }
    out
}

fn diffs_page(page: usize) -> String {
    let from = (page - 1) * 100;
    let to = (from + 100).min(FILES);
    let items: Vec<_> = (from..to)
        .map(|i| {
            json!({
                "old_path": path_of(i),
                "new_path": path_of(i),
                "a_mode": "100644",
                "b_mode": "100644",
                "diff": patch(i),
                "new_file": false,
                "renamed_file": false,
                "deleted_file": false
            })
        })
        .collect();
    serde_json::to_string(&items).unwrap()
}

/// The fixture instance with MR 21's diff replaced by `FILES` files and
/// their raw contents served; counts the raw reads.
pub fn huge() -> (String, Log, Arc<Mutex<usize>>) {
    let raws = Arc::new(Mutex::new(0usize));
    let seen = raws.clone();
    let (base, log) = serve(move |req: &Request| {
        let (route, query) = req.path.split_once('?').unwrap_or((&req.path, ""));
        if route == format!("{MR21}/diffs") {
            let page = query
                .split('&')
                .find_map(|p| p.strip_prefix("page="))
                .and_then(|p| p.parse::<usize>().ok())
                .unwrap_or(1);
            let next = if page * 100 < FILES {
                format!("X-Next-Page: {}\r\n", page + 1)
            } else {
                "X-Next-Page: \r\n".into()
            };
            return Reply {
                headers: next,
                ..reply("200 OK", diffs_page(page))
            };
        }
        if let Some(rest) = route.strip_prefix("/api/v4/projects/grp%2Fapp/repository/files/") {
            if let Some(enc) = rest.strip_suffix("/raw") {
                let path = enc.replace("%2F", "/");
                let i: usize = path
                    .rsplit("file")
                    .next()
                    .and_then(|s| s.strip_suffix(".rs"))
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                *seen.lock().unwrap() += 1;
                return reply("200 OK", text(i, query.contains(HEAD_SHA)));
            }
        }
        instance(req)
    });
    (base, log, raws)
}

pub fn core_over(base: &str) -> (Core, ReviewId) {
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![Arc::new(provider(base))]);
    assert!(core.refresh().errors.is_empty());
    let id = core.reviews().unwrap()[0].id;
    (core, id)
}

fn ms(t: Instant) -> u128 {
    t.elapsed().as_millis()
}

#[test]
// Ignored everywhere already, so no `cfg_attr(windows, ignore)` (a second
// `ignore` fails clippy on Windows); Unix only, like the fake glab.
#[ignore = "a measurement (Unix): run with --ignored --nocapture"]
fn big_review_timings() {
    let (base, log, raws) = huge();
    let (core, id) = core_over(&base);
    let requests = || log.lock().unwrap().len();

    let n0 = requests();
    let t = Instant::now();
    let files = core.list_files(id, false).unwrap();
    println!(
        "list_files (cold): {} files, {} ms, {} requests",
        files.files.len(),
        ms(t),
        requests() - n0
    );

    let n0 = requests();
    let t = Instant::now();
    let files2 = core.list_files(id, false).unwrap();
    assert_eq!(files2.files.len(), FILES);
    println!(
        "list_files (again): {} ms, {} requests",
        ms(t),
        requests() - n0
    );

    let n0 = requests();
    let t = Instant::now();
    for f in &files.files {
        core.commentable_lines(id, &f.file.path).unwrap();
    }
    println!(
        "commentable_lines x{FILES}: {} ms, {} requests",
        ms(t),
        requests() - n0
    );

    let sides: Vec<(String, String)> = files
        .files
        .iter()
        .flat_map(|f| {
            [
                (files.base_sha.clone(), f.file.path.clone()),
                (files.head_sha.clone(), f.file.path.clone()),
            ]
        })
        .collect();
    let r0 = *raws.lock().unwrap();
    let t = Instant::now();
    for (sha, path) in sides.iter().take(80) {
        core.file_at(id, sha, path).unwrap();
    }
    println!(
        "file_at x80 one by one: {} ms, {} raw reads",
        ms(t),
        *raws.lock().unwrap() - r0
    );
    let r0 = *raws.lock().unwrap();
    let t = Instant::now();
    for (sha, path) in sides.iter().take(80) {
        core.file_at(id, sha, path).unwrap();
    }
    println!(
        "file_at x80 again: {} ms, {} raw reads",
        ms(t),
        *raws.lock().unwrap() - r0
    );
    for round in ["cold", "warm"] {
        let r0 = *raws.lock().unwrap();
        let t = Instant::now();
        for chunk in sides[80..].chunks(32) {
            for r in core.files_at(id, chunk).unwrap() {
                r.unwrap();
            }
        }
        println!(
            "files_at x{} in batches of 32 ({round}): {} ms, {} raw reads",
            sides.len() - 80,
            ms(t),
            *raws.lock().unwrap() - r0
        );
    }
}

fn raws(n: &Arc<Mutex<usize>>) -> usize {
    *n.lock().unwrap()
}

#[test]
#[cfg_attr(windows, ignore)]
fn contents_are_read_once_per_commit_and_survive_a_restart() {
    let (base, _log, reads) = huge();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("argus.db");
    let core = Core::with_providers(Store::open(&db).unwrap(), vec![Arc::new(provider(&base))]);
    assert!(core.refresh().errors.is_empty());
    let id = core.reviews().unwrap()[0].id;
    let path = path_of(3);
    let first = core.file_at(id, HEAD_SHA, &path).unwrap();
    assert_eq!(first, text(3, true));
    assert_eq!(core.file_at(id, HEAD_SHA, &path).unwrap(), first);
    assert_eq!(raws(&reads), 1);
    // Another commit of the same path is another read.
    assert_eq!(core.file_at(id, BASE_SHA, &path).unwrap(), text(3, false));
    assert_eq!(raws(&reads), 2);
    drop(core);

    // Argus restarted: the cache is on disk.
    let core = Core::with_providers(Store::open(&db).unwrap(), vec![Arc::new(provider(&base))]);
    assert_eq!(core.file_at(id, HEAD_SHA, &path).unwrap(), first);
    assert_eq!(raws(&reads), 2);
    assert!(core.cache_stats().unwrap().entries >= 2);
    core.clear_cache().unwrap();
    core.file_at(id, HEAD_SHA, &path).unwrap();
    assert_eq!(raws(&reads), 3);
}

#[test]
#[cfg_attr(windows, ignore)]
fn the_file_list_is_read_once_per_head_and_commentable_lines_share_it() {
    let (base, log, _) = huge();
    let (core, id) = core_over(&base);
    let diff_reads = || {
        log.lock()
            .unwrap()
            .iter()
            .filter(|r| r.path.starts_with(&format!("{MR21}/diffs")))
            .count()
    };
    let before = diff_reads();
    let first = core.list_files(id, false).unwrap();
    let pages = diff_reads();
    // Every page (and the probe past a full last page).
    assert!(pages - before >= FILES.div_ceil(100));
    assert_eq!(core.list_files(id, false).unwrap(), first);
    for f in first.files.iter().take(20) {
        core.commentable_lines(id, &f.file.path).unwrap();
    }
    assert_eq!(diff_reads(), pages);
    // Refreshing the review asks for it fresh.
    core.refresh_review(id).unwrap();
    let refreshed = diff_reads();
    assert_eq!(core.list_files(id, false).unwrap(), first);
    assert_eq!(diff_reads() - refreshed, pages - before);
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_batch_answers_as_single_reads_do() {
    let (base, _, _) = huge();
    let (core, id) = core_over(&base);
    let wanted: Vec<(String, String)> = vec![
        (HEAD_SHA.into(), path_of(1)),
        (BASE_SHA.into(), path_of(1)),
        (HEAD_SHA.into(), path_of(2)),
        // Not a file of the repository: its own error, the rest answer.
        (HEAD_SHA.into(), "src/missing.rs".into()),
        (HEAD_SHA.into(), path_of(5)),
    ];
    let batch = core.files_at(id, &wanted).unwrap();
    assert_eq!(batch.len(), wanted.len());
    for ((sha, path), got) in wanted.iter().zip(&batch) {
        match core.file_at(id, sha, path) {
            Ok(text) => assert_eq!(got.as_ref().unwrap(), &text, "{path}"),
            Err(_) => assert!(got.is_err(), "{path}"),
        }
    }
    let paths: Vec<String> = (0..5).map(path_of).chain(["nope.rs".into()]).collect();
    let lines = core.commentable_lines_of(id, &paths);
    for (path, got) in paths.iter().zip(&lines) {
        match core.commentable_lines(id, path) {
            Ok(one) => assert_eq!(got.as_ref().unwrap(), &one, "{path}"),
            Err(_) => assert!(got.is_err(), "{path}"),
        }
    }
}

#[test]
#[cfg_attr(windows, ignore)]
fn the_cache_on_disk_never_holds_the_token() {
    let (base, _, _) = huge();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("argus.db");
    let core = Core::with_providers(Store::open(&db).unwrap(), vec![Arc::new(provider(&base))]);
    assert!(core.refresh().errors.is_empty());
    let id = core.reviews().unwrap()[0].id;
    let files = core.list_files(id, false).unwrap();
    let wanted: Vec<(String, String)> = files
        .files
        .iter()
        .take(10)
        .map(|f| (HEAD_SHA.to_string(), f.file.path.clone()))
        .collect();
    core.files_at(id, &wanted).unwrap();
    assert!(core.cache_stats().unwrap().entries > 10);
    drop(core);
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        let hay = String::from_utf8_lossy(&bytes);
        assert!(!hay.contains(TOKEN));
        assert!(!hay.contains("PRIVATE-TOKEN"));
    }
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_clone_answers_first_and_its_reads_are_not_cached() {
    use std::process::Command;
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let repo = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo.path())
            .env("GIT_AUTHOR_NAME", "fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.test")
            .env("GIT_COMMITTER_NAME", "fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.test")
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}");
        String::from_utf8(out.stdout).unwrap()
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(repo.path().join("src/module1")).unwrap();
    std::fs::write(repo.path().join(path_of(1)), "from the clone\n").unwrap();
    git(&["add", "."]);
    git(&["-c", "commit.gpgsign=false", "commit", "-q", "-m", "one"]);
    let sha = git(&["rev-parse", "HEAD"]).trim().to_string();

    let (base, _, reads) = huge();
    let (core, id) = core_over(&base);
    let record = core.reviews().unwrap()[0].clone();
    let mut settings = core.settings();
    settings.clone_paths.insert(
        format!("{}/{}", record.review.host, record.review.repo),
        repo.path().to_path_buf(),
    );
    core.update_settings(settings).unwrap();

    let wanted = vec![
        (sha.clone(), path_of(1)),
        // The clone lacks the platform's head: the API answers.
        (HEAD_SHA.to_string(), path_of(1)),
    ];
    let batch = core.files_at(id, &wanted).unwrap();
    assert_eq!(batch[0].as_deref().unwrap(), "from the clone\n");
    assert_eq!(batch[1].as_deref().unwrap(), text(1, true));
    assert_eq!(
        core.file_at(id, &sha, &path_of(1)).unwrap(),
        "from the clone\n"
    );
    assert_eq!(raws(&reads), 1);
    // Only the API's read went into the cache.
    assert_eq!(core.cache_stats().unwrap().entries, 1);
}
