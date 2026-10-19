//! The safety net behind the prompt's rule that the agent never repeats the
//! review: after parsing, a finding that says what an existing thread (any
//! author, open, resolved or outdated) or an already posted finding says
//! is dropped and counted.
//!
//! Two things must hold for a finding on a code thread: the same file and
//! side, its line within [`LINE_SLACK`] of the thread's lines (where the
//! thread was left, where the platform shows it now, and where the thread
//! history found it at head, which follows renames), and similar text
//! ([`NEAR_SIMILARITY`]). A thread on the merge request as a whole has no
//! lines, so only the text counts there, at the stricter
//! [`GLOBAL_SIMILARITY`]. A similar text far away from a thread is kept: the
//! same mistake in another place is a new point.
//!
//! Similarity is token based (lower-cased words, a crude suffix strip,
//! stop words and generic review words left out): the best of the Jaccard
//! index of the finding against the whole thread, and the share of one set
//! found in the other for the finding's title in a note and for a note in
//! the finding, the latter two only for sets of at least [`MIN_TOKENS`]
//! words. The overlap catches the common case of a
//! short comment ("panics on an empty list") whose point the finding's
//! longer text contains in full, which plain Jaccard dilutes.

use std::collections::HashSet;

use super::model::Finding;
use super::output::{RawFinding, RunOutput};
use super::prompt::ThreadSnapshot;
use crate::provider::Side;
use crate::store::StoredThread;

/// How many lines a finding may sit away from a thread's lines and still be
/// about them (a comment on the `if`, a finding on the line under it).
pub(crate) const LINE_SLACK: u32 = 2;
/// From this similarity on, a finding near a thread's lines repeats it.
pub(crate) const NEAR_SIMILARITY: f64 = 0.5;
/// From this similarity on, a finding repeats a thread on the merge request
/// as a whole (no location to narrow it down, so stricter).
pub(crate) const GLOBAL_SIMILARITY: f64 = 0.7;
/// The overlap coefficient only counts for word sets at least this big: two
/// shared words of a two-word title prove nothing.
pub(crate) const MIN_TOKENS: usize = 3;

/// Words that carry no point of their own: English function words and the
/// stock phrases of review comments.
const STOP_WORDS: &[&str] = &[
    "a",
    "about",
    "above",
    "after",
    "all",
    "also",
    "an",
    "and",
    "any",
    "are",
    "as",
    "at",
    "be",
    "been",
    "before",
    "being",
    "below",
    "but",
    "by",
    "can",
    "could",
    "did",
    "do",
    "does",
    "doing",
    "each",
    "either",
    "else",
    "even",
    "for",
    "from",
    "had",
    "has",
    "have",
    "here",
    "how",
    "if",
    "in",
    "into",
    "is",
    "it",
    "its",
    "just",
    "may",
    "might",
    "more",
    "most",
    "much",
    "must",
    "no",
    "nor",
    "not",
    "now",
    "of",
    "on",
    "once",
    "one",
    "only",
    "or",
    "other",
    "our",
    "out",
    "over",
    "own",
    "same",
    "so",
    "some",
    "such",
    "than",
    "that",
    "the",
    "their",
    "them",
    "then",
    "there",
    "these",
    "they",
    "this",
    "those",
    "though",
    "through",
    "to",
    "too",
    "under",
    "up",
    "us",
    "very",
    "was",
    "we",
    "were",
    "what",
    "when",
    "where",
    "whether",
    "which",
    "while",
    "who",
    "why",
    "will",
    "with",
    "would",
    "yet",
    "you",
    "your",
    "i",
    "me",
    "my",
    "he",
    "she",
    "him",
    "her",
    "let",
    "lets",
    "get",
    "gets",
    "got",
    "make",
    "makes",
    "think",
    "maybe",
    "perhaps",
    "please",
    "consider",
    "should",
    "probably",
    "instead",
    "still",
    "already",
    "again",
    "nit",
    "suggest",
    "suggestion",
    "looks",
    "look",
    "seems",
    "seem",
    "like",
    "need",
    "needs",
    "want",
    "wants",
    "use",
    "used",
    "using",
    "code",
    "line",
    "lines",
    "change",
    "changes",
    "changed",
    "fix",
    "issue",
    "problem",
    "case",
    "thing",
    "way",
    "new",
    "old",
    "currently",
    "never",
    "always",
    "ok",
    "okay",
    "thanks",
    "thank",
    "yes",
];

/// A word as compared: lower case, a common suffix off.
fn stem(word: &str) -> String {
    for suffix in ["ing", "ed", "es", "s", "e"] {
        if let Some(rest) = word.strip_suffix(suffix) {
            if rest.chars().count() >= 3 && !(suffix == "s" && rest.ends_with('s')) {
                return rest.to_string();
            }
        }
    }
    word.to_string()
}

/// The meaningful words of `text` (see the module docs).
pub(crate) fn tokens(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 2 && !w.chars().all(|c| c.is_ascii_digit()))
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .map(|w| stem(&w))
        .collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    a.intersection(b).count() as f64 / union as f64
}

/// `|a ∩ b| / |a|`, for a set `a` of at least [`MIN_TOKENS`] words.
fn contained(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.len() < MIN_TOKENS {
        return 0.0;
    }
    a.intersection(b).count() as f64 / a.len() as f64
}

/// Lines `start..=end` of `path` on `side`; `None` lines: the whole file
/// (a comment on a file, not a line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Place {
    pub path: String,
    pub side: Side,
    pub lines: Option<(u32, u32)>,
}

impl Place {
    fn covers(&self, f: &RawFinding) -> bool {
        if self.path != f.path || self.side != f.side {
            return false;
        }
        match self.lines {
            None => true,
            Some((start, end)) => {
                f.line.saturating_add(LINE_SLACK) >= start
                    && f.line <= end.saturating_add(LINE_SLACK)
            }
        }
    }
}

/// A point the review already makes: where (empty: the merge request as a
/// whole) and its words.
#[derive(Debug, Clone, Default)]
pub(crate) struct Raised {
    pub places: Vec<Place>,
    /// Per note (or per posted finding, its title and body), the words.
    pub notes: Vec<HashSet<String>>,
    /// All of them together.
    pub all: HashSet<String>,
}

impl Raised {
    fn new(places: Vec<Place>, texts: impl IntoIterator<Item = String>) -> Self {
        let notes: Vec<HashSet<String>> = texts
            .into_iter()
            .map(|t| tokens(&t))
            .filter(|t| !t.is_empty())
            .collect();
        let all = notes.iter().flatten().cloned().collect();
        Self { places, notes, all }
    }

    /// How much `f` says what this point says (0 to 1).
    pub fn similarity(&self, f: &RawFinding) -> f64 {
        let title = tokens(&f.title);
        let mut whole = tokens(&f.body);
        whole.extend(title.iter().cloned());
        // Per note for the overlaps: in a long discussion three title
        // words are found somewhere by chance.
        let notes = self
            .notes
            .iter()
            .map(|n| contained(&title, n).max(contained(n, &whole)))
            .fold(0.0, f64::max);
        jaccard(&whole, &self.all).max(notes)
    }

    /// Whether `f` repeats this point.
    pub fn repeated_by(&self, f: &RawFinding) -> bool {
        if self.places.is_empty() {
            return self.similarity(f) >= GLOBAL_SIMILARITY;
        }
        self.places.iter().any(|p| p.covers(f)) && self.similarity(f) >= NEAR_SIMILARITY
    }
}

/// The places of a thread: where it was left (on its side, the range of a
/// multi-line comment), where the platform shows it in the current diff
/// (`current_line`), and where the thread history located it at head
/// (`snapshot`, new side; follows renames). Each path a side may name
/// counts. Empty for a thread on the merge request as a whole.
pub(crate) fn thread_places(st: &StoredThread, snapshot: Option<&ThreadSnapshot>) -> Vec<Place> {
    let Some(a) = &st.thread.anchor else {
        return Vec::new();
    };
    let (line, paths) = match a.side {
        Side::New => (a.line_new, [&a.path_new, &a.path_old]),
        Side::Old => (a.line_old, [&a.path_old, &a.path_new]),
    };
    let span = match (a.start_line, line) {
        (Some(s), Some(l)) if s <= l => l - s,
        _ => 0,
    };
    let range = |end: u32| Some((end.saturating_sub(span), end));
    let mut ends: Vec<Option<(u32, u32)>> = vec![line.and_then(range)];
    if let Some(cur) = a.current_line {
        ends.push(range(cur));
    }
    let mut places = Vec::new();
    for path in paths.into_iter().flatten() {
        for lines in &ends {
            places.push(Place {
                path: path.clone(),
                side: a.side,
                lines: *lines,
            });
        }
    }
    if let Some(s) = snapshot {
        if let (Some(path), Some(l)) = (&s.now_path, s.now_line) {
            places.push(Place {
                path: path.clone(),
                side: Side::New,
                lines: range(l),
            });
        }
    }
    places.dedup();
    places
}

/// What the review already says: every thread (its human notes; system
/// notes are the platform's, not a point) and every finding the reviewer
/// posted (whose thread a refresh may not have fetched yet).
pub(crate) fn raised_points(
    threads: &[(&StoredThread, Option<ThreadSnapshot>)],
    posted: &[&Finding],
) -> Vec<Raised> {
    let mut out: Vec<Raised> = threads
        .iter()
        .map(|(st, snap)| {
            Raised::new(
                thread_places(st, snap.as_ref()),
                st.thread
                    .notes
                    .iter()
                    .filter(|n| !n.system)
                    .map(|n| n.body.clone()),
            )
        })
        .collect();
    out.extend(posted.iter().map(|f| {
        Raised::new(
            vec![Place {
                path: f.path.clone(),
                side: f.side,
                lines: Some((f.line, f.line)),
            }],
            [format!("{}\n{}", f.title, f.body)],
        )
    }));
    out
}

/// Drop the findings of `run` that repeat one of `raised`, counting them in
/// `dropped_duplicates`.
pub(crate) fn drop_repeats(run: &mut RunOutput, raised: &[Raised]) {
    let before = run.findings.len();
    run.findings
        .retain(|f| !raised.iter().any(|r| r.repeated_by(f)));
    run.dropped_duplicates += (before - run.findings.len()) as u32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::model::Severity;
    use crate::ids::{ReviewId, ThreadId};
    use crate::provider::Note;
    use crate::testutil::{at, note, thread, AUTHOR, ME};

    fn finding(path: &str, line: u32, title: &str, body: &str) -> RawFinding {
        RawFinding {
            path: path.into(),
            line,
            side: Side::New,
            severity: Severity::Medium,
            title: title.into(),
            body: body.into(),
        }
    }

    fn said(author: &str, body: &str) -> Note {
        Note {
            body: body.into(),
            ..note(&format!("n-{}", body.len()), author, at(1, 1))
        }
    }

    /// A thread by `author` on `src/cache.rs` line 10 (new side), saying
    /// `bodies`; `None` anchor for one on the merge request as a whole.
    fn stored(author: &str, bodies: &[&str], anchored: bool) -> StoredThread {
        let mut t = thread(
            "t1",
            author,
            "h1",
            bodies.iter().map(|b| said(author, b)).collect(),
        );
        if !anchored {
            t.anchor = None;
        }
        let review = ReviewId::from_key("r");
        StoredThread {
            thread_id: ThreadId::from_key(review, "t1"),
            review_id: review,
            mine: author == ME,
            thread: t,
        }
    }

    fn repeats(st: &StoredThread, snap: Option<ThreadSnapshot>, f: &RawFinding) -> bool {
        raised_points(&[(st, snap)], &[])[0].repeated_by(f)
    }

    const EMPTY_LIST: &str =
        "This panics when `items` is empty: `items[0]` has no guard. Check `is_empty()` first.";

    #[test]
    fn a_near_duplicate_on_the_same_or_an_adjacent_line_is_dropped() {
        let st = stored(AUTHOR, &[EMPTY_LIST], true);
        let f = |line| {
            finding(
                "src/cache.rs",
                line,
                "Indexing an empty items list panics",
                "`items[0]` panics when `items` is empty. Guard with `is_empty()` or use `first()`.",
            )
        };
        for line in [8, 10, 12] {
            assert!(repeats(&st, None, &f(line)), "line {line}");
        }
        // Beyond the slack it is another place.
        assert!(!repeats(&st, None, &f(13)));
        assert!(!repeats(&st, None, &f(7)));
    }

    #[test]
    fn a_different_issue_on_the_same_line_is_kept() {
        let st = stored(AUTHOR, &[EMPTY_LIST], true);
        let f = finding(
            "src/cache.rs",
            10,
            "TTL of 0 disables the cache",
            "With `ttl = 0` every lookup misses, so the cache never serves a hit. Default to 300.",
        );
        assert!(!repeats(&st, None, &f));
        // The same text on another file is kept too.
        let mut elsewhere = finding(
            "src/other.rs",
            10,
            "Indexing an empty items list panics",
            "`items[0]` panics when `items` is empty.",
        );
        assert!(!repeats(&st, None, &elsewhere));
        // And on the other side of the same file.
        elsewhere.path = "src/cache.rs".into();
        elsewhere.side = Side::Old;
        assert!(!repeats(&st, None, &elsewhere));
    }

    #[test]
    fn a_similar_text_on_a_far_away_line_is_kept() {
        let st = stored(AUTHOR, &[EMPTY_LIST], true);
        let f = finding(
            "src/cache.rs",
            80,
            "Indexing an empty items list panics",
            "`items[0]` panics when `items` is empty. Guard with `is_empty()`.",
        );
        assert!(!repeats(&st, None, &f));
    }

    #[test]
    fn a_paraphrase_of_a_comment_on_the_merge_request_is_dropped_anywhere() {
        let st = stored(
            "bob-fixture",
            &["The cache is never invalidated after a write, so readers see stale widgets until the TTL expires."],
            false,
        );
        let f = finding(
            "src/store.rs",
            42,
            "Writes never invalidate the cache",
            "After a write the cached widgets stay stale until the TTL expires; invalidate the entry on write.",
        );
        assert!(repeats(&st, None, &f));
        // Merely on the same subject is not enough without a place.
        let tests = stored(
            "bob-fixture",
            &["Please add tests for the cache invalidation path."],
            false,
        );
        let race = finding(
            "src/cache.rs",
            10,
            "Cache invalidation races with concurrent writes",
            "Two writers can interleave: the second write's invalidation runs before the first write lands, leaving a stale entry.",
        );
        assert!(!repeats(&tests, None, &race));
        let related = finding(
            "src/store.rs",
            42,
            "Cache key ignores the tenant",
            "Two tenants share widget cache entries because the key is the widget id only.",
        );
        assert!(!repeats(&st, None, &related));
    }

    #[test]
    fn replies_resolved_and_outdated_threads_count_and_the_thread_is_followed_to_head() {
        let mut st = stored(
            AUTHOR,
            &[
                "Why a Vec here?",
                "Lookups by id are linear in the widget count; a HashMap keyed by id avoids the scan.",
            ],
            true,
        );
        st.thread.resolved = true;
        st.thread.outdated = true;
        let f = |path: &str, line| {
            finding(
                path,
                line,
                "Linear lookup by widget id",
                "Each lookup scans the Vec; key a HashMap by the widget id.",
            )
        };
        assert!(repeats(&st, None, &f("src/cache.rs", 11)));
        // Renamed and moved since: the history's location at head counts,
        // and so does the platform's current line.
        let snap = ThreadSnapshot {
            now_path: Some("src/widget_cache.rs".into()),
            now_line: Some(40),
            ..ThreadSnapshot::default()
        };
        assert!(repeats(
            &st,
            Some(snap.clone()),
            &f("src/widget_cache.rs", 41)
        ));
        assert!(!repeats(&st, None, &f("src/widget_cache.rs", 41)));
        if let Some(a) = st.thread.anchor.as_mut() {
            a.current_line = Some(25);
        }
        assert!(repeats(&st, None, &f("src/cache.rs", 24)));
    }

    #[test]
    fn a_multi_line_comment_covers_its_whole_range() {
        let mut st = stored(AUTHOR, &[EMPTY_LIST], true);
        if let Some(a) = st.thread.anchor.as_mut() {
            a.start_line = Some(4);
        }
        let f = finding(
            "src/cache.rs",
            3,
            "Indexing an empty items list panics",
            "`items[0]` panics when `items` is empty.",
        );
        assert!(repeats(&st, None, &f));
    }

    #[test]
    fn a_posted_finding_counts_as_raised() {
        let posted = Finding {
            id: crate::ai::FindingId::new(),
            review_id: ReviewId::from_key("r"),
            round: 1,
            path: "src/a.rs".into(),
            line: 2,
            side: Side::New,
            severity: Severity::High,
            title: "TTL of 0 disables the cache".into(),
            body: "Every lookup misses.".into(),
            dismissed: false,
            posted_at: Some(at(2, 2)),
            posted_url: Some("https://example.test/1".into()),
        };
        let raised = raised_points(&[], &[&posted]);
        let mut run = RunOutput {
            findings: vec![
                finding(
                    "src/a.rs",
                    3,
                    "A TTL of 0 disables caching",
                    "Every lookup misses the cache.",
                ),
                finding(
                    "src/a.rs",
                    3,
                    "Unbounded cache growth",
                    "Nothing evicts entries.",
                ),
            ],
            ..RunOutput::default()
        };
        drop_repeats(&mut run, &raised);
        assert_eq!(run.findings.len(), 1);
        assert_eq!(run.findings[0].title, "Unbounded cache growth");
        assert_eq!(run.dropped_duplicates, 1);
    }

    #[test]
    fn system_notes_and_short_titles_prove_nothing() {
        let mut st = stored(ME, &["Looks fine."], true);
        st.thread.notes.push(Note {
            system: true,
            ..said(
                ME,
                "changed this line in version 2 of the diff: lookup cache entries",
            )
        });
        let f = finding(
            "src/cache.rs",
            10,
            "Cache entries lookup",
            "Lookup of cache entries happens twice per request.",
        );
        assert!(!repeats(&st, None, &f));
    }

    #[test]
    fn tokens_drop_stop_words_and_strip_suffixes() {
        let t = tokens("Please consider caching the widgets; the cache is used twice.");
        assert!(t.contains("cach"));
        assert!(t.contains("widget"));
        assert!(t.contains("twic"));
        assert!(!t.contains("please") && !t.contains("the") && !t.contains("used"));
        assert_eq!(tokens("class"), ["class".to_string()].into());
        assert!(tokens("a 42 x").is_empty());
    }
}
