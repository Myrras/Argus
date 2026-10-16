//! Integration tests: exercise the public `argus-core` API exactly as the Tauri
//! shell (or any other adapter) would — no UI required.

use argus_core::{Core, CoreError};

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
