use std::process::Command;

use super::super::tests::git_in;
use super::*;

/// A git repository at `root/rel` with `url` as its `origin`; the clone
/// search never looks at commits, so it stays empty.
fn repo_at(root: &Path, rel: &str, url: &str) -> PathBuf {
    let dir = root.join(rel);
    std::fs::create_dir_all(&dir).unwrap();
    git_in(&dir, &["init", "-q", "-b", "main"]);
    git_in(&dir, &["remote", "add", "origin", url]);
    dir
}

fn git_installed() -> bool {
    if Command::new("git").arg("--version").output().is_err() || git_binary().is_none() {
        eprintln!("skipping: git is not installed");
        return false;
    }
    true
}

/// A temp directory, canonical (macOS puts them under `/private`, and the
/// scan answers with the paths of the roots it was given).
fn temp_root() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(dir.path()).unwrap();
    (dir, path)
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_clone_is_found_by_its_remote_whatever_the_url_looks_like() {
    if !git_installed() {
        return;
    }
    for url in [
        "git@github.com:acme-fixture/widgets.git",
        "https://github.com/acme-fixture/widgets.git",
        "https://someone:glpat-fixture-token@github.com:8443/Acme-Fixture/Widgets",
        "ssh://git@github.com/acme-fixture/widgets",
    ] {
        let (_tmp, root) = temp_root();
        let want = repo_at(&root, "work/widgets", url);
        assert_eq!(
            find_clone(
                "github.com",
                "acme-fixture/widgets",
                std::slice::from_ref(&root)
            ),
            Some(want),
            "{url}"
        );
    }
}

#[test]
#[cfg_attr(windows, ignore)]
fn another_repository_a_plain_folder_and_another_host_are_no_match() {
    if !git_installed() {
        return;
    }
    let (_tmp, root) = temp_root();
    repo_at(&root, "work/other", "git@github.com:acme-fixture/other.git");
    repo_at(
        &root,
        "work/same-name",
        "git@gitlab.example.test:acme-fixture/widgets.git",
    );
    std::fs::create_dir_all(root.join("work/widgets")).unwrap();
    assert_eq!(
        find_clone("github.com", "acme-fixture/widgets", &[root]),
        None
    );
}

#[test]
#[cfg_attr(windows, ignore)]
fn nothing_deeper_than_three_levels_and_no_build_folder_is_looked_at() {
    if !git_installed() {
        return;
    }
    let url = "git@github.com:acme-fixture/widgets.git";
    let (_tmp, root) = temp_root();
    // Three levels under the root is as deep as it goes.
    let at_three = repo_at(&root, "a/b/widgets", url);
    assert_eq!(
        find_clone(
            "github.com",
            "acme-fixture/widgets",
            std::slice::from_ref(&root)
        ),
        Some(at_three)
    );
    let (_tmp, root) = temp_root();
    repo_at(&root, "a/b/c/widgets", url);
    assert_eq!(
        find_clone("github.com", "acme-fixture/widgets", &[root]),
        None
    );
    // A clone a dependency folder happens to hold is none of ours.
    let (_tmp, root) = temp_root();
    repo_at(&root, "app/node_modules/widgets", url);
    repo_at(&root, ".cache/widgets", url);
    assert_eq!(
        find_clone("github.com", "acme-fixture/widgets", &[root]),
        None
    );
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_repository_is_answered_for_but_never_walked_into() {
    if !git_installed() {
        return;
    }
    // A repository nested in another one: the outer one answers (it does
    // not match), and its working tree is not walked.
    let (_tmp, root) = temp_root();
    repo_at(&root, "work/outer", "git@github.com:acme-fixture/outer.git");
    repo_at(
        &root,
        "work/outer/inner",
        "git@github.com:acme-fixture/widgets.git",
    );
    assert_eq!(
        find_clone("github.com", "acme-fixture/widgets", &[root]),
        None
    );
}

#[test]
#[cfg_attr(windows, ignore)]
fn the_folder_offered_is_checked_to_be_that_repositorys_clone() {
    if !git_installed() {
        return;
    }
    let (_tmp, root) = temp_root();
    let dir = repo_at(&root, "widgets", "git@github.com:acme-fixture/widgets.git");
    assert!(is_clone_of("github.com", "acme-fixture/widgets", &dir));
    assert!(!is_clone_of("github.com", "acme-fixture/other", &dir));
    assert!(!is_clone_of(
        "gitlab.example.test",
        "acme-fixture/widgets",
        &dir
    ));
    // A folder that is no repository at all.
    let plain = root.join("plain");
    std::fs::create_dir(&plain).unwrap();
    assert!(!is_clone_of("github.com", "acme-fixture/widgets", &plain));
}

#[test]
fn the_search_looks_where_the_other_clones_are_and_in_the_usual_places() {
    let (_tmp, home) = temp_root();
    for rel in ["code", "Developer", "work/vcs", "elsewhere"] {
        std::fs::create_dir_all(home.join(rel)).unwrap();
    }
    let clones = [home.join("work/vcs/widgets"), home.join("gone/x")];
    let roots = search_roots(clones.iter().map(PathBuf::as_path), Some(&home));
    // The parent of a configured clone first, then the home folders that
    // exist (`~/src` does not here), each once, and never a folder that is
    // not there.
    assert_eq!(
        roots,
        vec![
            home.join("work/vcs"),
            home.join("code"),
            home.join("Developer"),
        ]
    );
}
