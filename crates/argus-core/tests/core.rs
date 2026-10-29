//! Integration tests: exercise the public `argus-core` API exactly as the Tauri
//! shell (or any other adapter) would — no UI required.

use argus_core::{AccountChoice, Core, CoreError, ReviewId, Settings};

#[test]
fn settings_persist_through_the_core() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("argus.db");
    let file = dir.path().join("settings.json");
    // Opening asks no CLI: that waits for the first account read.
    let core = Core::open(&db, &file).unwrap();
    assert_eq!(core.settings(), Settings::default());

    let mut s = core.settings();
    s.accounts.insert(
        " gitlab:GitLab.example.test ".into(),
        AccountChoice {
            enabled: false,
            account: Some("ignored".into()),
        },
    );
    s.poll_interval_secs = 300;
    let stored = core.update_settings(s).unwrap();
    assert_eq!(
        stored.accounts.into_iter().collect::<Vec<_>>(),
        [(
            "gitlab:gitlab.example.test".to_string(),
            AccountChoice {
                enabled: false,
                account: None
            }
        )]
    );

    let reopened = Core::open(&db, &file).unwrap();
    assert_eq!(reopened.settings(), core.settings());
    assert!(reopened.reviews().unwrap().is_empty());
    assert!(matches!(
        reopened.review(ReviewId::from_key("missing")),
        Err(CoreError::NotFound)
    ));
}

#[test]
fn an_in_memory_core_has_no_accounts() {
    // No CLI is asked: an in-memory core has no providers of its own.
    let core = Core::new();
    assert!(core.list_accounts(false).is_empty());
    assert!(core.list_accounts(true).is_empty());
    assert!(core.refresh().errors.is_empty());
}
