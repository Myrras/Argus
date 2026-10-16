//! Integration tests: exercise the public `argus-core` API exactly as the Tauri
//! shell (or any other adapter) would — no UI required.

use argus_core::{Core, CoreError, ReviewId, Settings};

#[test]
fn settings_persist_through_the_core() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("argus.db");
    let file = dir.path().join("settings.json");
    let core = Core::open(&db, &file).unwrap();
    assert_eq!(core.settings(), Settings::default());

    let mut s = core.settings();
    s.github.enabled = false;
    s.gitlab.base_url = "https://gitlab.example.test/".into();
    s.poll_interval_secs = 300;
    let stored = core.update_settings(s).unwrap();
    assert_eq!(stored.gitlab.base_url, "https://gitlab.example.test");

    let reopened = Core::open(&db, &file).unwrap();
    assert_eq!(reopened.settings(), stored);
    assert!(reopened.reviews().unwrap().is_empty());

    let mut bad = stored.clone();
    bad.gitlab.base_url = "not a url".into();
    assert!(matches!(
        reopened.update_settings(bad),
        Err(CoreError::InvalidInput(_))
    ));
    assert!(matches!(
        reopened.review(ReviewId::from_key("missing")),
        Err(CoreError::NotFound)
    ));
}

#[test]
fn provider_status_reflects_settings() {
    let core = Core::new();
    let mut s = core.settings();
    s.github.enabled = false;
    core.update_settings(s).unwrap();
    let status = core.provider_status();
    // Disabled: `gh` is not even probed.
    assert!(!status.github.enabled);
    assert!(!status.github.installed);
    assert!(!status.gitlab.configured);
    assert!(!status.gitlab.token_present);
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["github"]["loggedIn"], false);
    assert_eq!(json["gitlab"]["tokenPresent"], false);
}
