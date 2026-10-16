//! Conditional GETs: the last `ETag` / `Last-Modified` and body per URL, so a
//! poll that finds nothing new gets a `304 Not Modified` instead of the full
//! response again. On GitHub a 304 does not count against the rate limit.
//!
//! In memory only (a restart starts cold), per provider, bounded by entry
//! count and total body size; the least recently used entry goes first.
//! Keys are request paths, never a token.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// How many responses are kept per provider.
const MAX_ENTRIES: usize = 512;
/// Total body bytes kept per provider.
const MAX_BYTES: usize = 64 * 1024 * 1024;
/// A single body larger than this is not kept (one huge diff page would
/// otherwise push out everything else).
const MAX_ENTRY_BYTES: usize = 8 * 1024 * 1024;

/// One cached response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cached {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub body: String,
    /// What the response said about the next page (GitHub: the `Link`
    /// `rel="next"` query, GitLab: `X-Next-Page`), replayed on a 304.
    pub next: Option<String>,
}

impl Cached {
    /// The validator header to send: `If-None-Match` when there is an ETag,
    /// else `If-Modified-Since`. `None` when neither is usable as a header
    /// value (a server's odd value never becomes a broken request).
    pub fn validator(&self) -> Option<(&'static str, &str)> {
        let usable = |v: &&String| is_header_value(v);
        if let Some(etag) = self.etag.as_ref().filter(usable) {
            return Some(("If-None-Match", etag));
        }
        self.last_modified
            .as_ref()
            .filter(usable)
            .map(|lm| ("If-Modified-Since", lm.as_str()))
    }
}

/// Printable ASCII only: no CR/LF that could split a header.
fn is_header_value(v: &str) -> bool {
    !v.is_empty() && v.len() <= 512 && v.bytes().all(|b| (0x20..0x7f).contains(&b))
}

struct Entry {
    used: u64,
    value: Arc<Cached>,
}

#[derive(Default)]
struct Inner {
    map: HashMap<String, Entry>,
    tick: u64,
    bytes: usize,
}

/// A bounded LRU of [`Cached`] responses.
pub(crate) struct ConditionalCache {
    inner: Mutex<Inner>,
    max_entries: usize,
    max_bytes: usize,
}

impl Default for ConditionalCache {
    fn default() -> Self {
        Self::with_limits(MAX_ENTRIES, MAX_BYTES)
    }
}

impl ConditionalCache {
    pub fn with_limits(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            max_entries: max_entries.max(1),
            max_bytes,
        }
    }

    /// The cached response for `key`, marked as just used.
    pub fn get(&self, key: &str) -> Option<Arc<Cached>> {
        let mut inner = self.inner.lock().expect("cache lock poisoned");
        inner.tick += 1;
        let tick = inner.tick;
        let entry = inner.map.get_mut(key)?;
        entry.used = tick;
        Some(entry.value.clone())
    }

    /// Keep `value` for `key` when it has a validator; a response without one
    /// can never come back as a 304, so it only drops an older entry.
    pub fn put(&self, key: &str, value: Cached) {
        let mut inner = self.inner.lock().expect("cache lock poisoned");
        if let Some(old) = inner.map.remove(key) {
            inner.bytes -= old.value.body.len();
        }
        let size = value.body.len();
        if value.validator().is_none() || size > MAX_ENTRY_BYTES.min(self.max_bytes) {
            return;
        }
        while !inner.map.is_empty()
            && (inner.map.len() >= self.max_entries || inner.bytes + size > self.max_bytes)
        {
            // O(n) over at most a few hundred entries, once per insert past
            // the bound: cheaper than keeping a second ordered index.
            let Some(oldest) = inner
                .map
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(e) = inner.map.remove(&oldest) {
                inner.bytes -= e.value.body.len();
            }
        }
        inner.tick += 1;
        let used = inner.tick;
        inner.bytes += size;
        inner.map.insert(
            key.to_string(),
            Entry {
                used,
                value: Arc::new(value),
            },
        );
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner.lock().unwrap().map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tagged(body: &str) -> Cached {
        Cached {
            etag: Some(format!("W/\"{body}\"")),
            last_modified: None,
            body: body.into(),
            next: None,
        }
    }

    #[test]
    fn evicts_the_least_recently_used_by_count() {
        let c = ConditionalCache::with_limits(2, 1024);
        c.put("a", tagged("1"));
        c.put("b", tagged("2"));
        assert!(c.get("a").is_some()); // b is now the oldest
        c.put("c", tagged("3"));
        assert_eq!(c.len(), 2);
        assert!(c.get("b").is_none());
        assert_eq!(c.get("a").unwrap().body, "1");
        assert_eq!(c.get("c").unwrap().body, "3");
    }

    #[test]
    fn bounds_total_bytes_and_replaces_in_place() {
        let c = ConditionalCache::with_limits(10, 8);
        c.put("a", tagged("aaaa"));
        c.put("b", tagged("bbbb"));
        c.put("c", tagged("cc"));
        assert!(c.get("a").is_none());
        c.put("b", tagged("b"));
        assert_eq!(c.get("b").unwrap().body, "b");
        // Larger than the whole budget: not kept, and the old entry goes.
        c.put("c", tagged("ccccccccc"));
        assert!(c.get("c").is_none());
    }

    #[test]
    fn needs_a_usable_validator() {
        let c = ConditionalCache::default();
        c.put(
            "plain",
            Cached {
                etag: None,
                last_modified: None,
                body: "x".into(),
                next: None,
            },
        );
        assert!(c.get("plain").is_none());
        let bad = Cached {
            etag: Some("\"a\r\nX-Evil: 1\"".into()),
            last_modified: Some("Wed, 14 Oct 2026 10:00:00 GMT".into()),
            body: "x".into(),
            next: None,
        };
        assert_eq!(
            bad.validator(),
            Some(("If-Modified-Since", "Wed, 14 Oct 2026 10:00:00 GMT"))
        );
        assert_eq!(tagged("x").validator(), Some(("If-None-Match", "W/\"x\"")));
    }
}
