//! Strongly-typed UUID newtypes.
//!
//! A `uuid_newtype!` macro keeps ids distinct at the type level (you cannot pass
//! a `ThreadId` where a `ReviewId` is expected) while serializing transparently
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
    /// Identifies a review (one MR or PR). Derived from its natural key, so
    /// the same MR/PR always gets the same id across refreshes and restarts.
    ReviewId
);

uuid_newtype!(
    /// Identifies a thread on a review, derived from the review id and the
    /// platform's own thread id.
    ThreadId
);

/// Namespace for the name-based (v5) ids below. Fixed forever: changing it
/// would re-key every stored review.
const ARGUS_NAMESPACE: Uuid = Uuid::from_u128(0x6a1f_4c2e_9b0d_4e57_a3c8_52d1_7e90_b4f6);

impl ReviewId {
    /// The id for a natural key such as `github|github.com|acme/widgets|12`.
    pub fn from_key(key: &str) -> Self {
        Self(Uuid::new_v5(&ARGUS_NAMESPACE, key.as_bytes()))
    }
}

impl ThreadId {
    /// The id for the platform thread `provider_thread_id` on `review`.
    pub fn from_key(review: ReviewId, provider_thread_id: &str) -> Self {
        let key = format!("{review}|{provider_thread_id}");
        Self(Uuid::new_v5(&ARGUS_NAMESPACE, key.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_plain_uuid_string() {
        let id = ReviewId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
    }

    #[test]
    fn keyed_ids_are_stable_and_distinct() {
        assert_eq!(ReviewId::from_key("a"), ReviewId::from_key("a"));
        assert_ne!(ReviewId::from_key("a"), ReviewId::from_key("b"));
        let r = ReviewId::from_key("a");
        assert_eq!(ThreadId::from_key(r, "1"), ThreadId::from_key(r, "1"));
        assert_ne!(ThreadId::from_key(r, "1"), ThreadId::from_key(r, "2"));
    }

    #[test]
    fn round_trips_through_from_str() {
        let id = ReviewId::new();
        let parsed: ReviewId = id.to_string().parse().unwrap();
        assert_eq!(parsed, id);
    }
}
