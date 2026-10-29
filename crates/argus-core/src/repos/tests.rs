use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::*;
use crate::open::Opener;
use crate::provider::{
    DiffRange, FileDiff, ProviderId, ReviewDetail, ReviewProvider, ReviewRef, Thread,
};
use crate::settings::SkillSource;
use crate::store::Store;
use crate::testutil::{detail, review_ref, ME};

fn review(
    host: &str,
    repo: &str,
    provider: ProviderId,
    active: bool,
) -> (String, String, ProviderId, bool) {
    (host.into(), repo.into(), provider, active)
}

/// Every clone counts as found: the pure listing without the disk.
fn found(_: &Path) -> CloneState {
    CloneState::Found
}

#[test]
fn the_list_is_the_reviews_repositories_and_the_configured_ones_each_once() {
    let mut s = Settings::default();
    // The user's accounts: the provider of a settings-only repository.
    let hosts = [
        (ProviderId::Github, "github.com".to_string()),
        (ProviderId::Gitlab, "gitlab.example.test".to_string()),
    ];
    s.clone_paths.insert(
        "github.com/acme-fixture/widgets".into(),
        PathBuf::from("/src/widgets"),
    );
    s.review_skills.insert(
        "gitlab.example.test/acme-fixture/api".into(),
        ReviewSkillChoice::None,
    );
    s.review_languages.insert(
        "git.example.test/acme-fixture/tools".into(),
        ReviewLanguage::Dutch,
    );
    let reviews = vec![
        review(
            "github.com",
            "acme-fixture/widgets",
            ProviderId::Github,
            true,
        ),
        review(
            "github.com",
            "acme-fixture/widgets",
            ProviderId::Github,
            true,
        ),
        review(
            "github.com",
            "acme-fixture/widgets",
            ProviderId::Github,
            false,
        ),
        // Only closed or approved reviews: listed, no open ones.
        review(
            "github.com",
            "acme-fixture/Zebra",
            ProviderId::Github,
            false,
        ),
        review(
            "gitlab.example.test",
            "acme-fixture/group/web",
            ProviderId::Gitlab,
            true,
        ),
    ];
    let list = list(&s, &hosts, reviews, found);
    let keys: Vec<&str> = list.iter().map(|r| r.key.as_str()).collect();
    // Open reviews first, then by name whatever the case.
    assert_eq!(
        keys,
        [
            "github.com/acme-fixture/widgets",
            "gitlab.example.test/acme-fixture/group/web",
            "git.example.test/acme-fixture/tools",
            "github.com/acme-fixture/Zebra",
            "gitlab.example.test/acme-fixture/api",
        ]
    );
    let widgets = &list[0];
    assert_eq!(widgets.host, "github.com");
    assert_eq!(widgets.repo, "acme-fixture/widgets");
    assert_eq!(widgets.provider, Some(ProviderId::Github));
    assert_eq!(widgets.open_reviews, 2);
    assert_eq!(
        widgets.clone_path.as_deref(),
        Some(Path::new("/src/widgets"))
    );
    assert_eq!(widgets.clone, CloneState::Found);
    assert_eq!(widgets.review_skill, ReviewSkillChoice::Auto);
    assert_eq!(widgets.review_language, None);

    let web = &list[1];
    assert_eq!(web.repo, "acme-fixture/group/web");
    assert_eq!(web.clone, CloneState::NotSet);
    assert_eq!(web.clone_path, None);

    // Only in the settings: the provider is told by the account's host.
    let tools = &list[2];
    assert_eq!(
        (tools.host.as_str(), tools.repo.as_str()),
        ("git.example.test", "acme-fixture/tools")
    );
    assert_eq!(tools.provider, None);
    assert_eq!(tools.open_reviews, 0);
    assert_eq!(tools.review_language, Some(ReviewLanguage::Dutch));
    let api = &list[4];
    assert_eq!(api.provider, Some(ProviderId::Gitlab));
    assert_eq!(api.review_skill, ReviewSkillChoice::None);
    assert_eq!(list[3].open_reviews, 0);

    // Nothing stored, nothing configured: an empty list.
    assert!(super::list(&Settings::default(), &[], Vec::new(), found).is_empty());
}

#[test]
fn a_clone_is_found_missing_or_no_repository() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("widgets");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    assert_eq!(clone_state(&repo), CloneState::Found);
    // A worktree or submodule has a `.git` file: a repository too.
    let worktree = dir.path().join("wt");
    std::fs::create_dir(&worktree).unwrap();
    std::fs::write(worktree.join(".git"), "gitdir: /elsewhere\n").unwrap();
    assert_eq!(clone_state(&worktree), CloneState::Found);
    let plain = dir.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    assert_eq!(clone_state(&plain), CloneState::NotRepository);
    assert_eq!(clone_state(&dir.path().join("gone")), CloneState::Missing);
    let file = dir.path().join("file");
    std::fs::write(&file, "x").unwrap();
    assert_eq!(clone_state(&file), CloneState::Missing);
}

/// Lists whatever the test put in `refs` and serves a detail for each.
struct Listing {
    refs: Mutex<Vec<ReviewRef>>,
}

impl ReviewProvider for Listing {
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
        Ok(self.refs.lock().unwrap().clone())
    }
    fn fetch_review(&self, r: &ReviewRef) -> CoreResult<ReviewDetail> {
        Ok(ReviewDetail {
            review: r.clone(),
            ..detail("h1")
        })
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

#[test]
fn a_repository_seen_for_the_first_time_is_listed_after_the_refresh() {
    let provider = Arc::new(Listing {
        refs: Mutex::new(vec![review_ref(12)]),
    });
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![provider.clone()]);
    assert!(core.list_repositories().unwrap().is_empty());
    core.refresh();
    let list = core.list_repositories().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].key, "github.com/acme-fixture/widgets");
    assert_eq!(list[0].open_reviews, 1);
    assert_eq!(list[0].clone, CloneState::NotSet);

    let mut gadgets = review_ref(7);
    gadgets.repo = "acme-fixture/gadgets".into();
    gadgets.url = "https://github.com/acme-fixture/gadgets/pull/7".into();
    provider.refs.lock().unwrap().push(gadgets);
    core.refresh();
    let keys: Vec<String> = core
        .list_repositories()
        .unwrap()
        .into_iter()
        .map(|r| r.key)
        .collect();
    assert_eq!(
        keys,
        [
            "github.com/acme-fixture/gadgets",
            "github.com/acme-fixture/widgets"
        ]
    );

    // Its setup shows once saved: a choice, a language, a clone.
    let dir = tempfile::tempdir().unwrap();
    let mut s = core.settings();
    s.clone_paths
        .insert("github.com/acme-fixture/gadgets".into(), dir.path().into());
    s.review_skills.insert(
        "github.com/acme-fixture/gadgets".into(),
        ReviewSkillChoice::Skill {
            source: SkillSource::User,
            name: "strict-review".into(),
        },
    );
    s.review_languages.insert(
        "github.com/acme-fixture/gadgets".into(),
        ReviewLanguage::Other {
            name: "Deutsch".into(),
        },
    );
    core.update_settings(s).unwrap();
    let gadgets = &core.list_repositories().unwrap()[0];
    assert_eq!(gadgets.clone, CloneState::NotRepository);
    assert_eq!(gadgets.clone_path.as_deref(), Some(dir.path()));
    assert!(matches!(
        gadgets.review_skill,
        ReviewSkillChoice::Skill { .. }
    ));
    assert_eq!(
        gadgets.review_language,
        Some(ReviewLanguage::Other {
            name: "Deutsch".into()
        })
    );
}

#[derive(Default)]
struct FakeOpener {
    revealed: Mutex<Vec<PathBuf>>,
}

impl Opener for FakeOpener {
    fn open(&self, _: &Path, _: Option<&str>) -> CoreResult<()> {
        Ok(())
    }
    fn reveal(&self, path: &Path) -> CoreResult<()> {
        self.revealed.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }
}

#[test]
fn only_the_clone_set_for_a_repository_is_shown_in_the_file_manager() {
    let core = Core::new();
    let opener = Arc::new(FakeOpener::default());
    core.set_opener(opener.clone());
    let dir = tempfile::tempdir().unwrap();
    let mut s = core.settings();
    s.clone_paths
        .insert("github.com/acme-fixture/widgets".into(), dir.path().into());
    s.clone_paths.insert(
        "github.com/acme-fixture/gone".into(),
        dir.path().join("gone"),
    );
    core.update_settings(s).unwrap();

    core.reveal_repo_clone(" github.com/acme-fixture/widgets ")
        .unwrap();
    assert_eq!(
        *opener.revealed.lock().unwrap(),
        [std::fs::canonicalize(dir.path()).unwrap()]
    );
    for repo in [
        "github.com/acme-fixture/gone",
        "github.com/acme-fixture/none",
        "/etc",
    ] {
        assert!(
            matches!(
                core.reveal_repo_clone(repo),
                Err(CoreError::InvalidInput(_))
            ),
            "{repo}"
        );
    }
    assert_eq!(opener.revealed.lock().unwrap().len(), 1);
}
