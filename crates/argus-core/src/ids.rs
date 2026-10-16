//! Strongly-typed UUID newtypes.
//!
//! A `uuid_newtype!` macro keeps ids distinct at the type level (you cannot pass
//! a `NoteId` where some other id is expected) while serializing transparently
//! as a plain UUID string across the IPC boundary.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! uuid_newtype {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Generate a fresh random (v4) id.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::from_str(s)?))
            }
        }
    };
}

uuid_newtype!(
    /// Identifies a note owned by the [`Core`](crate::Core).
    NoteId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_plain_uuid_string() {
        let id = NoteId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
    }

    #[test]
    fn round_trips_through_from_str() {
        let id = NoteId::new();
        let parsed: NoteId = id.to_string().parse().unwrap();
        assert_eq!(parsed, id);
    }
}
