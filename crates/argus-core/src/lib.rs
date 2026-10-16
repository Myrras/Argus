//! argus-core — the UI-agnostic heart of Argus, a review cockpit for
//! GitLab merge requests and GitHub pull requests.
//!
//! Owns all domain logic and state behind a single entry point, [`Core`], and
//! has **zero** dependency on Tauri (or any UI). The Tauri shell (`src-tauri`)
//! is a thin adapter over [`Core`]'s public API, which means the whole domain is
//! testable on plain `cargo test` without spinning up a window.
//!
//! ## The core contract
//! All mutation flows through [`Core`]. Keep orchestration/domain logic here;
//! the shell should only (de)serialize arguments and forward calls. This keeps
//! the core compiling — and testable — without the UI.
//!
//! The greet + notes sample (inherited from the boilerplate) keeps the
//! core → IPC → frontend wiring tested until the real domain modules land.

#![forbid(unsafe_code)]

pub mod error;
pub mod ids;

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

pub use error::{CoreError, CoreResult};
pub use ids::NoteId;

/// One note. Serializable so it can cross the IPC boundary unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Note {
    pub id: NoteId,
    pub text: String,
}

/// The application's single entry point.
///
/// Cheap to share: hold it behind an `Arc` in the shell and clone the handle
/// into any detached task. Interior mutability ([`Mutex`]) keeps the public API
/// `&self`, so callers never need a mutable borrow.
#[derive(Default)]
pub struct Core {
    notes: Mutex<Vec<Note>>,
}

impl Core {
    pub fn new() -> Self {
        Self::default()
    }

    /// A pure, fallible sample command: greet `name`, rejecting blank input.
    ///
    /// Demonstrates input validation returning [`CoreError::InvalidInput`],
    /// which the shell maps to a stable IPC error `kind`.
    pub fn greet(&self, name: &str) -> CoreResult<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::InvalidInput("name must not be empty".into()));
        }
        Ok(format!("Hello, {name}! Welcome to Argus."))
    }

    /// Add a note and return it.
    pub fn add_note(&self, text: &str) -> CoreResult<Note> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CoreError::InvalidInput(
                "note text must not be empty".into(),
            ));
        }
        let note = Note {
            id: NoteId::new(),
            text: text.to_owned(),
        };
        self.notes
            .lock()
            .expect("notes lock poisoned")
            .push(note.clone());
        Ok(note)
    }

    /// All notes, in insertion order.
    pub fn list_notes(&self) -> Vec<Note> {
        self.notes.lock().expect("notes lock poisoned").clone()
    }

    /// Remove a note by id, erroring if it does not exist.
    pub fn remove_note(&self, id: NoteId) -> CoreResult<()> {
        let mut notes = self.notes.lock().expect("notes lock poisoned");
        let before = notes.len();
        notes.retain(|n| n.id != id);
        if notes.len() == before {
            return Err(CoreError::NotFound);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greet_rejects_blank_names() {
        let core = Core::new();
        assert!(core.greet("   ").is_err());
        assert_eq!(core.greet("Ada").unwrap(), "Hello, Ada! Welcome to Argus.");
    }

    #[test]
    fn add_then_remove_note() {
        let core = Core::new();
        let note = core.add_note("first").unwrap();
        assert_eq!(core.list_notes().len(), 1);
        core.remove_note(note.id).unwrap();
        assert!(core.list_notes().is_empty());
        // Removing a missing id is an error, not a silent no-op.
        assert!(matches!(
            core.remove_note(note.id),
            Err(CoreError::NotFound)
        ));
    }
}
