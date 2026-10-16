//! Integration tests: exercise the public `argus-core` API exactly as the Tauri
//! shell (or any other adapter) would — no UI required.

use argus_core::{Core, CoreError, ReviewId, Settings};

#[test]
fn notes_round_trip_through_the_public_api() {
    let core = Core::new();
    assert!(core.list_notes().is_empty());

    let a = core.add_note("buy milk").unwrap();
    let b = core.add_note("write tests").unwrap();

    let notes = core.list_notes();
    assert_eq!(notes.len(), 2);
    // Insertion order is preserved.
    assert_eq!(notes[0].id, a.id);
    assert_eq!(notes[1].id, b.id);

    core.remove_note(a.id).unwrap();
    let notes = core.list_notes();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].id, b.id);
}

#[test]
fn invalid_input_is_rejected() {
    let core = Core::new();
    assert!(matches!(
        core.add_note("   "),
        Err(CoreError::InvalidInput(_))
    ));
    assert!(matches!(core.greet(""), Err(CoreError::InvalidInput(_))));
}

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
