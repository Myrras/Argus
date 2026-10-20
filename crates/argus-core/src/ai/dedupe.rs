//! Keeps the AI pass from making a point the review already makes. After
//! parsing, each finding and checklist item is compared with every point
//! raised so far: each thread (any author, open, resolved or outdated; its
//! human notes) and each finding the user posted whose thread was not
//! fetched yet. A finding or item that repeats one is not shown as such: it
//! is stored with the point it repeats and why ([`Repeat`]), listed apart
//! ("Skipped as already raised"), and the user can still show it.
//!
//! Two signals, both on prose words only (see below):
//!
//! 1. The agent's own: every finding and item lists the ids of the
//!    threads it overlaps (`relatedThreadIds`, only ids the prompt listed).
//!    The prompt tells it to leave out a point that adds nothing and to
//!    describe only the new part of one that does; so a marked point
//!    repeats its thread only when the text also says what the thread says
//!    ([`RELATED_SIMILARITY`]): a related point the agent kept anyway is
//!    caught, a new part on the same code is kept. A high-severity finding
//!    is never set aside on the agent's mark alone (an over-eager or
//!    steered mark must not bury it): it stays a finding and names what it
//!    relates to. So does every other marked finding that is kept.
//! 2. A net of Argus' own for a point the agent did not mark: the same
//!    file and side, its line within [`LINE_SLACK`] of the thread's lines
//!    (where it was left, where the platform shows it now, where the thread
//!    history found it at head), and similar words ([`NEAR_SIMILARITY`]).
//!    On the other side of the same file: at a line the thread's hunk maps
//!    to, the same rule; when the thread has no place on that side at all,
//!    only a near copy ([`GLOBAL_SIMILARITY`]). A thread on the merge
//!    request as a whole has no lines: the text alone at
//!    [`GLOBAL_SIMILARITY`]. A similar text far away from a thread on the
//!    same side is a new point (the same mistake in another place).
//!
//! Only prose words count: code says what the line is, not what is wrong
//! with it. Left out are fenced blocks (code, suggestions), inline code
//! spans, identifier-shaped words (`snake_case`, `camelCase`, with digits)
//! and every word of the code the point is on (the thread's hunk end and
//! the finding's own line). Words are lower-cased and stemmed (English and
//! Dutch suffixes, repeatedly), and then English and Dutch stop words and
//! stock review phrases are left out (by stem, so `fixed` goes with
//! `fix`). At least [`MIN_SHARED_WORDS`] words must be shared.
//!
//! Similarity is the best of the Jaccard index of the point against the
//! whole thread, the share of the finding's title found in a note (titles
//! of at least [`MIN_TOKENS`] words), and the share of a note found in the
//! finding. The last one catches a comment whose point a longer finding
//! contains in full, which plain Jaccard dilutes; it counts only for a note
//! of at least [`MIN_NOTE_WORDS`] words whose words the finding's title
//! shares too: three words of a short note ("the cache never expires
//! entries") are its subject, and a finding about something else on that
//! subject mentions them in its body.

use std::collections::HashSet;
use std::sync::LazyLock;

use super::model::{Finding, FindingId, RelatedRef, RepeatOf, RepeatReason, Severity};
use super::output::{RawCheck, RawFinding, RunOutput};
use super::prompt::ThreadSnapshot;
use crate::ids::ThreadId;
use crate::provider::{FileDiff, Side};
use crate::store::StoredThread;

/// How many lines a finding may sit away from a thread's lines and still be
/// about them (a comment on the `if`, a finding on the line under it).
pub(crate) const LINE_SLACK: u32 = 2;
/// From this word similarity on, a finding near a thread's lines repeats it.
pub(crate) const NEAR_SIMILARITY: f64 = 0.5;
/// From this word similarity on, a point repeats a thread on the merge
/// request as a whole, or one on the other side of its file (no line to
/// narrow it down, so stricter).
pub(crate) const GLOBAL_SIMILARITY: f64 = 0.7;
/// From this word similarity on, a point the agent marked as related to a
/// thread still says what the thread says (prose words only, as the net:
/// shared code on the same line is what a new part has too).
pub(crate) const RELATED_SIMILARITY: f64 = 0.3;
/// The share of a title found in a note only counts for titles at least
/// this big: two shared words of a two-word title prove nothing.
pub(crate) const MIN_TOKENS: usize = 3;
/// The share of a note found in a finding only counts for notes at least
/// this big (see the module docs).
pub(crate) const MIN_NOTE_WORDS: usize = 5;
/// The net needs at least this many shared words: one shared word ("cache")
/// is a subject, not a point.
pub(crate) const MIN_SHARED_WORDS: usize = 2;
/// Lines of a thread's hunk end (and of its then-code) taken as the code
/// it is on.
const CODE_LINES: usize = 3;

/// Words that carry no point of their own: English and Dutch function words
/// and the stock phrases of review comments in both. Compared by stem too.
const STOP_WORDS: &[&str] = &[
    // English function words.
    "a",
    "about",
    "above",
    "after",
    "again",
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
    "between",
    "both",
    "but",
    "by",
    "can",
    "cannot",
    "could",
    "did",
    "do",
    "does",
    "doing",
    "done",
    "down",
    "each",
    "either",
    "else",
    "even",
    "ever",
    "every",
    "for",
    "from",
    "had",
    "has",
    "have",
    "having",
    "he",
    "her",
    "here",
    "him",
    "his",
    "how",
    "i",
    "if",
    "in",
    "into",
    "is",
    "it",
    "its",
    "itself",
    "just",
    "me",
    "may",
    "might",
    "more",
    "most",
    "much",
    "must",
    "my",
    "no",
    "nor",
    "not",
    "now",
    "of",
    "off",
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
    "she",
    "should",
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
    "until",
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
    "whom",
    "why",
    "will",
    "with",
    "would",
    "yet",
    "you",
    "your",
    // English review phrases.
    "already",
    "always",
    "agree",
    "agreed",
    "addressed",
    "change",
    "changed",
    "changes",
    "code",
    "comment",
    "comments",
    "consider",
    "currently",
    "fix",
    "fixed",
    "fixes",
    "get",
    "gets",
    "got",
    "good",
    "great",
    "instead",
    "issue",
    "issues",
    "let",
    "lets",
    "lgtm",
    "like",
    "line",
    "lines",
    "look",
    "looks",
    "make",
    "makes",
    "maybe",
    "need",
    "needs",
    "never",
    "new",
    "nice",
    "nit",
    "ok",
    "okay",
    "old",
    "perhaps",
    "please",
    "probably",
    "problem",
    "resolved",
    "right",
    "seem",
    "seems",
    "still",
    "suggest",
    "suggestion",
    "sure",
    "thank",
    "thanks",
    "thing",
    "things",
    "think",
    "update",
    "updated",
    "use",
    "used",
    "using",
    "want",
    "wants",
    "way",
    "yes",
    "case",
    // Dutch function words.
    "aan",
    "al",
    "alle",
    "alles",
    "als",
    "ben",
    "bij",
    "dan",
    "dat",
    "de",
    "deze",
    "die",
    "dit",
    "door",
    "dus",
    "een",
    "eens",
    "elk",
    "elke",
    "en",
    "er",
    "geen",
    "had",
    "heb",
    "hebben",
    "heeft",
    "het",
    "hier",
    "hij",
    "hoe",
    "hun",
    "ik",
    "in",
    "is",
    "je",
    "jij",
    "kan",
    "kun",
    "kunnen",
    "maar",
    "met",
    "mij",
    "mijn",
    "moet",
    "moeten",
    "na",
    "naar",
    "niet",
    "niets",
    "nog",
    "nu",
    "of",
    "om",
    "omdat",
    "ons",
    "onze",
    "ook",
    "op",
    "over",
    "te",
    "toch",
    "tot",
    "u",
    "uit",
    "van",
    "veel",
    "voor",
    "waar",
    "want",
    "was",
    "wat",
    "we",
    "wel",
    "werd",
    "wie",
    "wij",
    "wordt",
    "worden",
    "ze",
    "zij",
    "zijn",
    "zo",
    "zou",
    "zouden",
    "iets",
    "ieder",
    "tussen",
    "zonder",
    "tegen",
    "onder",
    "boven",
    "daar",
    "waarom",
    "erg",
    "heel",
    "meer",
    "minder",
    "nooit",
    "altijd",
    "ervoor",
    "hierbij",
    // Dutch review phrases.
    "aangepast",
    "bedankt",
    "beter",
    "dank",
    "denk",
    "eigenlijk",
    "gedaan",
    "gefixt",
    "gebruik",
    "gebruiken",
    "gewoon",
    "goed",
    "graag",
    "inderdaad",
    "klopt",
    "liever",
    "lijkt",
    "misschien",
    "opgelost",
    "opmerking",
    "prima",
    "punt",
    "regel",
    "regels",
    "suggestie",
    "top",
    "vind",
    "wijziging",
    "wijzigingen",
    "zie",
];

static STOP: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| STOP_WORDS.iter().copied().collect());
/// Stems of the longer stop words, so their inflections go too (`fixed`,
/// `issues`). Short function words only match as written: their stems
/// (`dat`, `op`) are stems of real words (`date`, `ops`).
static STOP_STEMS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    STOP_WORDS
        .iter()
        .filter(|w| w.chars().count() >= 4)
        .map(|w| stem(w))
        .collect()
});

/// Suffixes stripped, longest first, until none applies (English and Dutch
/// inflections; `tokens`/`token` and `ontbreken`/`ontbreekt` end alike).
const SUFFIXES: &[&str] = &[
    "ations", "ation", "ating", "ated", "ates", "ate", "ments", "ment", "ingen", "heden", "heid",
    "ings", "ing", "edly", "ies", "ied", "ed", "es", "en", "er", "ly", "s", "e", "t", "y",
];

/// A word as compared: lower case, suffixes off, Dutch double vowels single.
pub(crate) fn stem(word: &str) -> String {
    let mut w = word.to_lowercase();
    loop {
        let stripped = SUFFIXES.iter().find_map(|suffix| {
            let rest = w.strip_suffix(suffix)?;
            (rest.chars().count() >= 3 && !(*suffix == "s" && rest.ends_with('s')))
                .then(|| rest.to_string())
        });
        match stripped {
            Some(rest) => w = rest,
            None => break,
        }
    }
    for (double, single) in [("ee", "e"), ("aa", "a"), ("oo", "o"), ("uu", "u")] {
        w = w.replace(double, single);
    }
    w
}

fn is_stop(lower: &str, stemmed: &str) -> bool {
    STOP.contains(lower) || STOP_STEMS.contains(stemmed)
}

/// A word written like code: `snake_case`, `camelCase`, with a digit.
fn identifier(w: &str) -> bool {
    w.contains('_')
        || w.chars().any(|c| c.is_ascii_digit())
        || w.chars()
            .zip(w.chars().skip(1))
            .any(|(a, b)| a.is_lowercase() && b.is_uppercase())
}

/// The words of `s`: letters, digits and `_`, two characters or more, not
/// a plain number.
fn words_of(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .map(|w| w.trim_matches('_'))
        .filter(|w| w.chars().count() >= 2 && !w.chars().all(|c| c.is_ascii_digit()))
}

/// A text as compared: its prose words, as written. Fenced blocks (code,
/// suggestions) and inline code spans are left out.
#[derive(Debug, Clone, Default)]
pub(crate) struct Text {
    prose: Vec<String>,
}

impl Text {
    pub fn new(s: &str) -> Self {
        let mut t = Text::default();
        let mut fenced = false;
        for line in s.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                fenced = !fenced;
                continue;
            }
            if fenced {
                continue;
            }
            // Odd segments between backticks are inline code.
            for seg in line.split('`').step_by(2) {
                t.prose.extend(words_of(seg).map(str::to_string));
            }
        }
        t
    }

    fn extend(&mut self, other: &Text) {
        self.prose.extend(other.prose.iter().cloned());
    }

    /// The comparable words of this text (stemmed) given `code` (lower-cased
    /// words of the code the point is on, which do not count).
    fn words(&self, code: &HashSet<String>) -> HashSet<String> {
        self.prose
            .iter()
            .filter(|w| !identifier(w))
            .filter_map(|w| {
                let lower = w.to_lowercase();
                let stemmed = stem(&lower);
                (!is_stop(&lower, &stemmed) && !code.contains(&lower)).then_some(stemmed)
            })
            .collect()
    }
}

/// Words of code lines, lower-cased: what a point on them cannot be judged by.
fn code_words<'a>(lines: impl IntoIterator<Item = &'a str>) -> HashSet<String> {
    lines
        .into_iter()
        .flat_map(|l| {
            words_of(diff_content(l))
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A diff line without its `+`/`-`/` ` marker.
fn diff_content(line: &str) -> &str {
    line.strip_prefix(['+', '-', ' ']).unwrap_or(line)
}

fn shared(a: &HashSet<String>, b: &HashSet<String>) -> usize {
    a.intersection(b).count()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    shared(a, b) as f64 / union as f64
}

/// `|a ∩ b| / |a|`, for a set `a` of at least `min` words.
fn contained(a: &HashSet<String>, b: &HashSet<String>, min: usize) -> f64 {
    if a.len() < min {
        return 0.0;
    }
    shared(a, b) as f64 / a.len() as f64
}

/// How much a point (`title` and `whole`, title included) says what
/// `notes` say (see the module docs), counting only overlaps of at least
/// [`MIN_SHARED_WORDS`] words.
fn similarity(title: &HashSet<String>, whole: &HashSet<String>, notes: &[HashSet<String>]) -> f64 {
    let all: HashSet<String> = notes.iter().flatten().cloned().collect();
    let mut best = 0.0_f64;
    if shared(whole, &all) >= MIN_SHARED_WORDS {
        best = jaccard(whole, &all);
    }
    for n in notes {
        if shared(whole, n) < MIN_SHARED_WORDS || shared(title, n) < MIN_SHARED_WORDS {
            continue;
        }
        best = best
            .max(contained(n, whole, MIN_NOTE_WORDS))
            .max(contained(title, n, MIN_TOKENS));
    }
    best
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
    fn near(&self, path: &str, side: Side, line: u32) -> bool {
        if self.path != path || self.side != side {
            return false;
        }
        match self.lines {
            None => true,
            Some((start, end)) => {
                line.saturating_add(LINE_SLACK) >= start && line <= end.saturating_add(LINE_SLACK)
            }
        }
    }
}

/// What a point already raised is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RaisedRef {
    Thread(ThreadId),
    /// A finding the user posted whose thread was not fetched yet.
    Posted(FindingId),
}

impl RaisedRef {
    /// The id the prompt lists it by (`relatedThreadIds` names it so).
    pub fn id(&self) -> String {
        match self {
            RaisedRef::Thread(t) => t.to_string(),
            RaisedRef::Posted(f) => f.to_string(),
        }
    }

    /// As the wire names it.
    pub fn related_ref(&self) -> RelatedRef {
        match *self {
            RaisedRef::Thread(t) => RelatedRef {
                thread_id: Some(t),
                finding_id: None,
            },
            RaisedRef::Posted(f) => RelatedRef {
                thread_id: None,
                finding_id: Some(f),
            },
        }
    }
}

/// Why a finding repeats the review, and what it repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Repeat {
    pub reason: RepeatReason,
    pub of: RaisedRef,
}

impl Repeat {
    pub fn repeat_of(&self) -> RepeatOf {
        let RelatedRef {
            thread_id,
            finding_id,
        } = self.of.related_ref();
        RepeatOf {
            reason: self.reason,
            thread_id,
            finding_id,
        }
    }
}

/// A point the review already makes: what it is, where (empty: the merge
/// request as a whole), its notes and the code it is on.
#[derive(Debug, Clone)]
pub(crate) struct Raised {
    pub of: RaisedRef,
    pub places: Vec<Place>,
    /// Per human note (or per posted finding, its title and body).
    notes: Vec<Text>,
    /// Words of the code it was left on (its hunk end, its then-code).
    code: HashSet<String>,
}

/// A point the run makes: a finding (with its place and the text of its
/// line) or a checklist item (no place).
pub(crate) struct Candidate<'a> {
    place: Option<(&'a str, Side, u32)>,
    /// A high-severity finding: never set aside on the agent's mark alone.
    high: bool,
    title: Text,
    whole: Text,
    code: HashSet<String>,
}

impl<'a> Candidate<'a> {
    /// `line_text`: the diff's text at the finding's line, when known.
    pub fn finding(f: &'a RawFinding, line_text: Option<&str>) -> Self {
        let title = Text::new(&f.title);
        let mut whole = Text::new(&f.body);
        whole.extend(&title);
        Candidate {
            place: Some((f.path.as_str(), f.side, f.line)),
            high: f.severity == Severity::High,
            title,
            whole,
            code: code_words(line_text),
        }
    }

    pub fn check(c: &'a RawCheck) -> Self {
        let title = Text::new(&c.text);
        Candidate {
            place: None,
            high: false,
            whole: title.clone(),
            title,
            code: HashSet::new(),
        }
    }
}

impl Raised {
    fn new(of: RaisedRef, places: Vec<Place>, texts: Vec<String>, code: HashSet<String>) -> Self {
        Self {
            of,
            places,
            notes: texts.iter().map(|t| Text::new(t)).collect(),
            code,
        }
    }

    /// How much `c` says what this point says: prose words only, the
    /// words of the code either is on left out.
    pub fn score(&self, c: &Candidate<'_>) -> f64 {
        let code: HashSet<String> = self.code.union(&c.code).cloned().collect();
        let notes: Vec<HashSet<String>> = self.notes.iter().map(|n| n.words(&code)).collect();
        similarity(&c.title.words(&code), &c.whole.words(&code), &notes)
    }

    /// Whether `c` repeats this point, and why (`related`: the agent listed
    /// this point's id for it).
    pub fn judge(&self, c: &Candidate<'_>, related: bool) -> Option<(RepeatReason, f64)> {
        let words = self.score(c);
        if related && !c.high && words >= RELATED_SIMILARITY {
            return Some((RepeatReason::AgentMarked, words));
        }
        let similar = match c.place {
            _ if self.places.is_empty() => words >= GLOBAL_SIMILARITY,
            None => words >= GLOBAL_SIMILARITY,
            Some((path, side, line)) => {
                let near = self.places.iter().any(|p| p.near(path, side, line));
                // A thread with a place on the finding's side is "near" or
                // nothing there: far down the same side is another place,
                // even when the thread's hunk also maps it across.
                let own_side = self.places.iter().any(|p| p.path == path && p.side == side);
                let other_side = !own_side && self.places.iter().any(|p| p.path == path);
                (near && words >= NEAR_SIMILARITY) || (other_side && words >= GLOBAL_SIMILARITY)
            }
        };
        similar.then_some((RepeatReason::Similar, words))
    }
}

/// The text at `line` on `side` of a unified diff `patch`.
pub(crate) fn line_at(patch: &str, side: Side, line: u32) -> Option<&str> {
    let (mut old, mut new) = (0u32, 0u32);
    for l in patch.lines() {
        if let Some((o, n)) = hunk_start(l) {
            (old, new) = (o, n);
            continue;
        }
        let (at_old, at_new) = match l.chars().next() {
            Some(' ') | None => (Some(old), Some(new)),
            Some('-') => (Some(old), None),
            Some('+') => (None, Some(new)),
            _ => continue,
        };
        let here = match side {
            Side::Old => at_old,
            Side::New => at_new,
        };
        if here == Some(line) {
            return Some(diff_content(l));
        }
        old += u32::from(at_old.is_some());
        new += u32::from(at_new.is_some());
    }
    None
}

/// `-a,b +c,d` of a hunk header: the first old and new line numbers.
fn hunk_start(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let new = rest.split_once(' ').map_or(rest, |(n, _)| n);
    let first = |s: &str| s.split(',').next()?.parse::<u32>().ok();
    Some((first(old)?, first(new)?))
}

/// The old and new line of a hunk excerpt's last line (where a GitHub
/// comment is): both for a context line, the neighbouring line on the
/// other side for an added or removed one.
fn hunk_end(hunk: &str) -> Option<(u32, u32)> {
    let (mut old, mut new) = (0u32, 0u32);
    let mut last = None;
    for l in hunk.lines() {
        if let Some((o, n)) = hunk_start(l) {
            (old, new) = (o, n);
            continue;
        }
        match l.chars().next() {
            Some(' ') | None => {
                last = Some((old, new));
                old += 1;
                new += 1;
            }
            Some('-') => {
                last = Some((old, new));
                old += 1;
            }
            Some('+') => {
                last = Some((old, new));
                new += 1;
            }
            _ => {}
        }
    }
    last
}

/// The last `n` lines of `s`.
fn last_lines(s: &str, n: usize) -> Vec<&str> {
    let lines: Vec<&str> = s.lines().filter(|l| !l.starts_with("@@")).collect();
    lines[lines.len().saturating_sub(n)..].to_vec()
}

/// The places of a thread: where it was left (on its side, the range of a
/// multi-line comment), where the platform shows it in the current diff
/// (`current_line`), and where the thread history located it at head
/// (`snapshot`, new side; follows renames). The same lines on the other
/// side count too when known: both lines of the anchor (a GitLab comment
/// on a context line), or the line its hunk excerpt maps to. Each path a
/// side may name counts. Empty for a thread on the merge request as a
/// whole.
pub(crate) fn thread_places(st: &StoredThread, snapshot: Option<&ThreadSnapshot>) -> Vec<Place> {
    let Some(a) = &st.thread.anchor else {
        return Vec::new();
    };
    let (line, other_line, other_side) = match a.side {
        Side::New => (a.line_new, a.line_old, Side::Old),
        Side::Old => (a.line_old, a.line_new, Side::New),
    };
    let span = match (a.start_line, line) {
        (Some(s), Some(l)) if s <= l => l - s,
        _ => 0,
    };
    let range = |end: u32| Some((end.saturating_sub(span), end));
    let mut ends: Vec<(Side, Option<(u32, u32)>)> = vec![(a.side, line.and_then(range))];
    if let Some(cur) = a.current_line {
        ends.push((a.side, range(cur)));
    }
    let mapped = a
        .diff_hunk
        .as_deref()
        .and_then(hunk_end)
        .map(|(o, n)| match other_side {
            Side::Old => o,
            Side::New => n,
        });
    for l in [other_line, mapped].into_iter().flatten() {
        ends.push((other_side, range(l)));
    }
    let mut places = Vec::new();
    for (side, lines) in ends {
        let paths = match side {
            Side::New => [&a.path_new, &a.path_old],
            Side::Old => [&a.path_old, &a.path_new],
        };
        for path in paths.into_iter().flatten() {
            places.push(Place {
                path: path.clone(),
                side,
                lines,
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
    let mut unique: Vec<Place> = Vec::new();
    for p in places {
        if !unique.contains(&p) {
            unique.push(p);
        }
    }
    unique
}

/// The code a thread was left on: the end of its hunk excerpt and of its
/// then-code.
fn thread_code(st: &StoredThread, snapshot: Option<&ThreadSnapshot>) -> HashSet<String> {
    let hunk = st
        .thread
        .anchor
        .as_ref()
        .and_then(|a| a.diff_hunk.as_deref())
        .map(|h| last_lines(h, CODE_LINES))
        .unwrap_or_default();
    let then = snapshot
        .and_then(|s| s.then_code.as_deref())
        .map(|c| last_lines(c, CODE_LINES))
        .unwrap_or_default();
    code_words(hunk.into_iter().chain(then))
}

/// What the review already says: every thread (its human notes; system
/// notes are the platform's, not a point) and every finding the reviewer
/// posted whose thread is not among them yet.
pub(crate) fn raised_points(
    threads: &[(&StoredThread, Option<ThreadSnapshot>)],
    posted: &[&Finding],
) -> Vec<Raised> {
    let mut out: Vec<Raised> = threads
        .iter()
        .map(|(st, snap)| {
            Raised::new(
                RaisedRef::Thread(st.thread_id),
                thread_places(st, snap.as_ref()),
                st.thread
                    .notes
                    .iter()
                    .filter(|n| !n.system)
                    .map(|n| n.body.clone())
                    .collect(),
                thread_code(st, snap.as_ref()),
            )
        })
        .collect();
    out.extend(posted.iter().map(|f| {
        Raised::new(
            RaisedRef::Posted(f.id),
            vec![Place {
                path: f.path.clone(),
                side: f.side,
                lines: Some((f.line, f.line)),
            }],
            vec![format!("{}\n{}", f.title, f.body)],
            HashSet::new(),
        )
    }));
    out
}

/// The point of `raised` that `c` repeats, if any: one the agent marked
/// first, then the closest.
fn best_repeat(c: &Candidate<'_>, related: &[String], raised: &[Raised]) -> Option<Repeat> {
    raised
        .iter()
        .filter_map(|r| {
            let marked = related.contains(&r.of.id());
            r.judge(c, marked)
                .map(|(reason, score)| (reason, score, r.of))
        })
        .max_by(|a, b| {
            let rank = |r: RepeatReason| u8::from(r == RepeatReason::AgentMarked);
            rank(a.0).cmp(&rank(b.0)).then(a.1.total_cmp(&b.1))
        })
        .map(|(reason, _, of)| Repeat { reason, of })
}

/// The text of `f`'s line in `diff` (its file's patch), if it is there.
fn finding_line<'d>(diff: &'d FileDiff, f: &RawFinding) -> Option<&'d str> {
    let file = diff
        .files
        .iter()
        .find(|c| c.path == f.path || c.old_path.as_deref() == Some(f.path.as_str()))?;
    line_at(file.patch.as_deref()?, f.side, f.line)
}

/// Mark the findings and checklist items of `run` that repeat one of
/// `raised` (counted in `dropped_duplicates` and `skipped_checklist`), and
/// name on each finding the points the agent said it relates to. `diff` is
/// the diff the agent read: the text of a finding's line is code, not a
/// point.
pub(crate) fn classify(run: &mut RunOutput, raised: &[Raised], diff: &FileDiff) {
    for f in &mut run.findings {
        let c = Candidate::finding(f, finding_line(diff, f));
        f.repeat = best_repeat(&c, &f.related, raised);
        f.related_to = raised
            .iter()
            .filter(|r| f.related.contains(&r.of.id()))
            .map(|r| r.of)
            .collect();
    }
    run.dropped_duplicates = run.findings.iter().filter(|f| f.repeat.is_some()).count() as u32;
    for item in &mut run.checklist {
        item.repeat = best_repeat(&Candidate::check(item), &item.related, raised);
    }
    run.skipped_checklist = run.checklist.iter().filter(|c| c.repeat.is_some()).count() as u32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::model::Severity;
    use crate::ids::{ReviewId, ThreadId};
    use crate::provider::{ChangedFile, FileStatus, Note};
    use crate::testutil::{at, note, thread, AUTHOR, ME};

    fn finding(path: &str, line: u32, title: &str, body: &str) -> RawFinding {
        RawFinding {
            path: path.into(),
            line,
            side: Side::New,
            severity: Severity::Medium,
            title: title.into(),
            body: body.into(),
            related: Vec::new(),
            repeat: None,
            related_to: Vec::new(),
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
            started_by_me: author == ME,
            thread: t,
        }
    }

    fn judge(
        st: &StoredThread,
        snap: Option<ThreadSnapshot>,
        f: &RawFinding,
        marked: bool,
    ) -> Option<RepeatReason> {
        raised_points(&[(st, snap)], &[])[0]
            .judge(&Candidate::finding(f, None), marked)
            .map(|(r, _)| r)
    }

    /// The net alone (the agent marked nothing).
    fn repeats(st: &StoredThread, snap: Option<ThreadSnapshot>, f: &RawFinding) -> bool {
        judge(st, snap, f, false).is_some()
    }

    const EMPTY_LIST: &str =
        "This panics when `items` is empty: `items[0]` has no guard. Check `is_empty()` first.";

    #[test]
    fn a_near_duplicate_on_the_same_or_an_adjacent_line_is_a_repeat() {
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
            assert_eq!(
                judge(&st, None, &f(line), false),
                Some(RepeatReason::Similar),
                "line {line}"
            );
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
        // On the other side of the file, at no line the thread maps to, a
        // merely similar text is kept (only a near copy is a repeat there).
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
    fn a_paraphrase_of_a_comment_on_the_merge_request_is_a_repeat_anywhere() {
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
            "`items[0]` panics when `items` is empty. Guard with `is_empty()`.",
        );
        assert!(repeats(&st, None, &f));
    }

    fn diff(path: &str, patch: &str) -> FileDiff {
        FileDiff {
            base_sha: "b".into(),
            head_sha: "h".into(),
            files: vec![ChangedFile {
                path: path.into(),
                old_path: None,
                status: FileStatus::Modified,
                additions: 1,
                deletions: 1,
                patch: Some(patch.into()),
                content_sha: "c".into(),
                binary: false,
                too_large: false,
            }],
            truncated: false,
            base_approximate: false,
        }
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
            repeat_of: None,
            related_to: Vec::new(),
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
        classify(&mut run, &raised, &diff("src/a.rs", "@@ -1 +1 @@\n-a\n+b"));
        assert_eq!(
            run.findings[0].repeat,
            Some(Repeat {
                reason: RepeatReason::Similar,
                of: RaisedRef::Posted(posted.id),
            })
        );
        assert_eq!(run.findings[1].repeat, None);
        assert_eq!(run.dropped_duplicates, 1);
        assert_eq!(
            run.findings[0].repeat.unwrap().repeat_of(),
            RepeatOf {
                reason: RepeatReason::Similar,
                thread_id: None,
                finding_id: Some(posted.id),
            }
        );
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

    // Review r10 probe cases: the old net dropped these (0.60, 1.00, 0.75).

    #[test]
    fn probe_a_naming_nit_does_not_swallow_an_auth_finding() {
        let st = stored(
            AUTHOR,
            &["Rename `user_id` to `owner_id` in `load_widget`."],
            true,
        );
        let f = finding(
            "src/cache.rs",
            10,
            "Missing authorization check in load_widget",
            "`load_widget` returns any widget by `user_id` without checking the caller owns it; compare `owner_id` with the session user.",
        );
        assert!(!repeats(&st, None, &f));
    }

    #[test]
    fn probe_a_suggestion_block_does_not_swallow_a_ttl_finding() {
        let st = stored(
            AUTHOR,
            &["```suggestion\nlet ttl = Duration::from_secs(300);\n```"],
            true,
        );
        let f = finding(
            "src/cache.rs",
            10,
            "TTL of 0 disables the cache",
            "With `ttl = 0` every lookup misses; default to `Duration::from_secs(300)`.",
        );
        assert!(!repeats(&st, None, &f));
        // Even when the agent marks it as related: the text says more.
        assert_eq!(judge(&st, None, &f, true), None);
    }

    #[test]
    fn probe_review_filler_words_prove_nothing() {
        let st = stored(AUTHOR, &["Thanks, fixed. Issues resolved."], true);
        let f = finding(
            "src/cache.rs",
            10,
            "Retry loop keeps firing after the first request resolved",
            "The fixed-interval retry issues a new request even after the first one resolved; stop the loop on success.",
        );
        assert!(!repeats(&st, None, &f));
    }

    // Review loop 2 round 2 probes: a new part on the same code (what the
    // prompt asks for), the same mistake far down the same side, and short
    // notes on another subject. The old rules set all of them aside.

    /// A comment asking for a `HashMap` on a lookup line (its hunk ends on
    /// it), and the text of that line.
    const LOOKUP_NOTE: &str = "Use a `HashMap` instead of the `Vec` for `widgets` lookups by id.";
    const LOOKUP_HUNK: &str = "@@ -8,3 +8,3 @@\n fn find(&self, id: u64) -> Option<&Widget> {\n-        self.widgets.get(id)\n+        self.widgets.iter().find(|w| w.id == id)";
    const LOOKUP_LINE: &str = "        self.widgets.iter().find(|w| w.id == id)";

    /// New parts on the lookup line: memory, a panic, a tenant bypass.
    const NEW_PARTS: [(&str, &str); 3] = [
        (
            "`widgets` grows without bound",
            "Besides the lookup cost already raised: every insert pushes onto the `Vec` of `widgets` and nothing evicts, so memory grows per request.",
        ),
        (
            "`find` result is unwrapped by the caller",
            "New beyond the lookup thread: the caller unwraps the `Option` from `find`, so an unknown widget id panics the request handler.",
        ),
        (
            "Widget lookup skips the tenant check",
            "Separate from the HashMap point: `find` returns any tenant's widget by id, so one tenant can read another's widgets.",
        ),
    ];

    fn with_hunk(mut st: StoredThread, hunk: &str) -> StoredThread {
        if let Some(a) = st.thread.anchor.as_mut() {
            a.diff_hunk = Some(hunk.into());
        }
        st
    }

    fn of_severity(f: RawFinding, severity: Severity) -> RawFinding {
        RawFinding { severity, ..f }
    }

    #[test]
    fn probe_new_parts_on_the_same_code_are_kept_even_when_marked() {
        let st = with_hunk(stored(AUTHOR, &[LOOKUP_NOTE], true), LOOKUP_HUNK);
        let raised = raised_points(&[(&st, None)], &[]);
        for (title, body) in NEW_PARTS {
            for severity in [Severity::High, Severity::Medium, Severity::Low] {
                let f = of_severity(finding("src/cache.rs", 10, title, body), severity);
                let c = Candidate::finding(&f, Some(LOOKUP_LINE));
                assert_eq!(raised[0].judge(&c, true), None, "{title} ({severity:?})");
                assert_eq!(raised[0].judge(&c, false), None, "{title} ({severity:?})");
            }
        }
    }

    #[test]
    fn probe_the_same_mistake_far_down_the_same_side_is_kept() {
        let hunk = "@@ -8,3 +8,3 @@\n ctx\n ctx2\n+let first = items[0];";
        let far = finding(
            "src/cache.rs",
            80,
            "Indexing an empty items list panics",
            "`items[0]` panics when `items` is empty. Guard with `is_empty()` or use `first()`.",
        );
        let st = with_hunk(stored(AUTHOR, &[EMPTY_LIST], true), hunk);
        // The hunk maps the thread to the old side too; the finding is on
        // the thread's own side, so only "near" counts there.
        assert!(!repeats(&st, None, &far));
        assert!(!repeats(&stored(AUTHOR, &[EMPTY_LIST], true), None, &far));
        // On the old side, where the thread's hunk also places it: the same.
        let old = RawFinding {
            side: Side::Old,
            ..far.clone()
        };
        assert!(!repeats(&st, None, &old));
        // Near it, it still repeats.
        assert!(repeats(&st, None, &RawFinding { line: 10, ..far }));
    }

    #[test]
    fn probe_short_notes_on_another_subject_do_not_swallow_high_findings() {
        let cases = [
            (
                "Please add a test for the token refresh path.",
                "Token is logged in plain text",
                "The token is logged by the debug call below; the refresh path leaks it.",
            ),
            (
                "The cache never expires entries.",
                "Cache entries race on concurrent writes",
                "Two writers to the cache can interleave so entries are lost; the cache needs a lock.",
            ),
        ];
        for (note, title, body) in cases {
            let st = stored(AUTHOR, &[note], false);
            let anchored = stored(AUTHOR, &[note], true);
            let f = of_severity(finding("src/cache.rs", 10, title, body), Severity::High);
            assert!(!repeats(&st, None, &f), "{title}");
            assert!(!repeats(&anchored, None, &f), "{title}");
            assert_eq!(judge(&anchored, None, &f, true), None, "{title}");
        }
        // A short note whose point the finding's title makes still repeats.
        let st = stored(AUTHOR, &["Panics on an empty list."], true);
        let f = finding(
            "src/cache.rs",
            10,
            "Indexing an empty items list panics",
            "`items[0]` panics when `items` is empty.",
        );
        assert!(repeats(&st, None, &f));
    }

    #[test]
    fn a_high_finding_is_never_set_aside_on_the_agents_mark_alone() {
        let st = stored(
            AUTHOR,
            &["Lookups by id are linear in the widget count; a HashMap keyed by id avoids the scan."],
            true,
        );
        // Far from the thread: only the agent's mark could set it aside.
        let far = finding(
            "src/cache.rs",
            40,
            "Linear lookup by widget id",
            "Each lookup scans the Vec; key a HashMap by the widget id.",
        );
        assert!(!repeats(&st, None, &far));
        assert_eq!(
            judge(&st, None, &far, true),
            Some(RepeatReason::AgentMarked)
        );
        let high = of_severity(far, Severity::High);
        assert_eq!(judge(&st, None, &high, true), None);
        // Near the thread the net still sets it aside, whatever its severity.
        let near = RawFinding { line: 10, ..high };
        assert_eq!(judge(&st, None, &near, true), Some(RepeatReason::Similar));
    }

    #[test]
    fn marked_findings_kept_name_what_they_relate_to() {
        let st = stored(AUTHOR, &[LOOKUP_NOTE], true);
        let raised = raised_points(&[(&st, None)], &[]);
        let mut f = of_severity(
            finding("src/cache.rs", 10, NEW_PARTS[2].0, NEW_PARTS[2].1),
            Severity::High,
        );
        f.related = vec![st.thread_id.to_string()];
        let unmarked = finding(
            "src/cache.rs",
            30,
            "Unbounded cache growth",
            "Nothing evicts.",
        );
        let mut run = RunOutput {
            findings: vec![f, unmarked],
            ..RunOutput::default()
        };
        classify(&mut run, &raised, &diff("src/none.rs", ""));
        assert_eq!(run.findings[0].repeat, None);
        assert_eq!(
            run.findings[0].related_to,
            vec![RaisedRef::Thread(st.thread_id)]
        );
        assert!(run.findings[1].related_to.is_empty());
        assert_eq!(run.dropped_duplicates, 0);
    }

    #[test]
    fn words_of_the_code_the_point_is_on_prove_nothing() {
        // The note and the finding share only words of the commented line.
        let mut st = stored(
            AUTHOR,
            &["Should the retry budget come from the config here?"],
            true,
        );
        if let Some(a) = st.thread.anchor.as_mut() {
            a.diff_hunk = Some("@@ -8,2 +8,3 @@\n ctx\n+let retry_budget = config.retry_budget;\n+let budget = retry * config;".into());
        }
        let f = finding(
            "src/cache.rs",
            10,
            "Retry budget from config is never validated",
            "A negative retry budget in the config makes the loop spin forever; reject it on load.",
        );
        let raised = raised_points(&[(&st, None)], &[]);
        let s = raised[0].score(&Candidate::finding(&f, None));
        assert!(s < NEAR_SIMILARITY, "{s}");
        assert!(!repeats(&st, None, &f));
    }

    #[test]
    fn dutch_comments_are_compared_without_their_stop_words() {
        let st = stored(
            "bob-fixture",
            &["De cache wordt nooit geïnvalideerd na een schrijfactie, dus lezers zien verouderde widgets tot de TTL verloopt."],
            true,
        );
        let repeat = finding(
            "src/cache.rs",
            10,
            "Cache wordt niet geïnvalideerd na schrijven",
            "Na een schrijfactie blijven verouderde widgets in de cache staan tot de TTL verloopt.",
        );
        assert!(repeats(&st, None, &repeat));
        // Dutch function words alone are no point.
        let other = finding(
            "src/cache.rs",
            10,
            "De sleutel van de cache negeert de tenant",
            "Twee tenants delen dus een widget in de cache omdat de sleutel alleen het id is.",
        );
        assert!(!repeats(&st, None, &other));
    }

    #[test]
    fn an_english_finding_on_a_dutch_comment_shares_no_words_and_is_kept() {
        let st = stored(
            "bob-fixture",
            &["Dit crasht als `items` leeg is: `items[0]` heeft geen check. Gebruik eerst `is_empty()`."],
            true,
        );
        let f = finding(
            "src/cache.rs",
            10,
            "Indexing an empty items list panics",
            "`items[0]` panics when `items` is empty; guard with `is_empty()` first.",
        );
        // Different languages share no prose words: the net cannot tell,
        // and shared code is no proof (a new part has it too). Marked, it
        // is kept and names the thread (`RawFinding::related_to`).
        assert!(!repeats(&st, None, &f));
        assert_eq!(judge(&st, None, &f, true), None);
    }

    #[test]
    fn a_marked_point_repeats_only_with_shared_prose_and_a_new_part_is_kept() {
        let st = stored(
            AUTHOR,
            &["Lookups by id are linear in the widget count; a HashMap keyed by id avoids the scan."],
            true,
        );
        let paraphrase = finding(
            "src/cache.rs",
            10,
            "Finding a widget walks the whole Vec",
            "Every call iterates all widgets to match the id; index them in a map instead.",
        );
        // Two shared words of eight: too little even with the mark (kept,
        // naming the thread).
        assert!(!repeats(&st, None, &paraphrase));
        assert_eq!(judge(&st, None, &paraphrase, true), None);
        // A closer wording far from the thread: the mark sets it aside.
        let far = finding(
            "src/cache.rs",
            40,
            "Lookups by widget id are linear",
            "Every lookup scans the widgets; a HashMap keyed by id avoids the scan.",
        );
        assert!(!repeats(&st, None, &far));
        assert_eq!(
            judge(&st, None, &far, true),
            Some(RepeatReason::AgentMarked)
        );
        // Marked related, but only the new part described: kept.
        let new_part = finding(
            "src/cache.rs",
            10,
            "The widget map is never cleared on logout",
            "Entries of a logged-out user stay in memory for good; clear them in `logout`.",
        );
        assert_eq!(judge(&st, None, &new_part, true), None);
    }

    #[test]
    fn the_other_side_counts_where_the_thread_maps_to() {
        // A comment on a removed line (old 12); its hunk maps it to new 12.
        let mut st = stored(AUTHOR, &[EMPTY_LIST], true);
        if let Some(a) = st.thread.anchor.as_mut() {
            a.side = Side::Old;
            a.line_old = Some(12);
            a.line_new = None;
            a.diff_hunk = Some("@@ -10,3 +10,2 @@\n ctx\n ctx2\n-let first = items[0];".into());
        }
        let f = |line| {
            finding(
                "src/cache.rs",
                line,
                "Indexing an empty items list panics",
                "`items[0]` panics when `items` is empty. Guard with `is_empty()` or use `first()`.",
            )
        };
        assert!(repeats(&st, None, &f(11)));
        // A looser text: a repeat at the mapped line, a new point far away.
        let loose = |line| {
            finding(
                "src/cache.rs",
                line,
                "Empty input panics here",
                "Nothing stops `items[0]` on an empty list.",
            )
        };
        assert!(repeats(&st, None, &loose(11)));
        assert!(!repeats(&st, None, &loose(30)));
        // Without a mapping only a near copy counts on the other side.
        if let Some(a) = st.thread.anchor.as_mut() {
            a.diff_hunk = None;
        }
        assert!(!repeats(&st, None, &f(11)));
        let copy = finding(
            "src/cache.rs",
            40,
            "Panics when items is empty",
            "This panics when `items` is empty: `items[0]` has no guard. Check `is_empty()` first.",
        );
        assert!(repeats(&st, None, &copy));
        // A GitLab comment on a context line has both lines.
        if let Some(a) = st.thread.anchor.as_mut() {
            a.line_new = Some(11);
        }
        assert!(repeats(&st, None, &f(11)));
    }

    #[test]
    fn checklist_items_that_repeat_a_thread_are_set_aside() {
        let st = stored(
            "bob-fixture",
            &["No migration for old caches: existing entries keep the old format."],
            false,
        );
        let raised = raised_points(&[(&st, None)], &[]);
        let check = |text: &str, related: bool| RawCheck {
            text: text.into(),
            related: if related {
                vec![st.thread_id.to_string()]
            } else {
                Vec::new()
            },
            repeat: None,
        };
        let mut run = RunOutput {
            checklist: vec![
                check(
                    "Is there a migration for old caches? Existing entries keep the old format.",
                    false,
                ),
                check("Is turning the cache off intended?", false),
                check("Do old cache entries need a migration on upgrade?", true),
                check("Was the rollout of the cache planned?", true),
            ],
            ..RunOutput::default()
        };
        classify(&mut run, &raised, &diff("src/none.rs", ""));
        // Kept, all of them; the repeats marked (listed apart, never lost).
        let marks: Vec<_> = run
            .checklist
            .iter()
            .map(|c| c.repeat.map(|r| r.reason))
            .collect();
        assert_eq!(
            marks,
            vec![
                Some(RepeatReason::Similar),
                None,
                Some(RepeatReason::AgentMarked),
                None
            ]
        );
        assert_eq!(run.skipped_checklist, 2);
    }

    #[test]
    fn the_agents_mark_wins_over_the_net_and_names_the_thread() {
        let st = stored(AUTHOR, &[EMPTY_LIST], true);
        let raised = raised_points(&[(&st, None)], &[]);
        let mut marked = finding(
            "src/cache.rs",
            10,
            "Indexing an empty items list panics",
            "`items[0]` panics when `items` is empty. Guard with `is_empty()`.",
        );
        marked.related = vec![st.thread_id.to_string()];
        let unmarked = RawFinding {
            related: Vec::new(),
            ..marked.clone()
        };
        let mut run = RunOutput {
            findings: vec![marked, unmarked],
            ..RunOutput::default()
        };
        classify(&mut run, &raised, &diff("src/none.rs", ""));
        let of = RaisedRef::Thread(st.thread_id);
        assert_eq!(
            run.findings[0].repeat,
            Some(Repeat {
                reason: RepeatReason::AgentMarked,
                of
            })
        );
        assert_eq!(
            run.findings[1].repeat,
            Some(Repeat {
                reason: RepeatReason::Similar,
                of
            })
        );
        assert_eq!(run.dropped_duplicates, 2);
    }

    #[test]
    fn the_findings_own_line_is_code() {
        let d = diff(
            "src/cache.rs",
            "@@ -9,3 +9,3 @@\n ctx\n-let ttl = 10;\n+let ttl = config.default_ttl;\n tail",
        );
        assert_eq!(
            line_at(&d.files[0].patch.clone().unwrap(), Side::New, 10),
            Some("let ttl = config.default_ttl;")
        );
        let patch = d.files[0].patch.clone().unwrap();
        assert_eq!(line_at(&patch, Side::Old, 10), Some("let ttl = 10;"));
        assert_eq!(line_at(&patch, Side::New, 11), Some("tail"));
        assert_eq!(line_at(&patch, Side::Old, 11), Some("tail"));
        assert_eq!(line_at(&patch, Side::New, 40), None);
        assert_eq!(hunk_end("@@ -10,3 +10,2 @@\n a\n b\n-c"), Some((12, 12)));
        assert_eq!(hunk_end("@@ -10,2 +10,3 @@\n a\n+b"), Some((11, 11)));
        assert_eq!(hunk_end("no hunk"), None);
    }

    #[test]
    fn words_are_stemmed_before_stop_words_go() {
        let t = Text::new(
            "Please consider caching the widgets; the cache is used twice. Fixed, issues resolved. `HashMap` user_id\n```\nlet hidden = 1;\n```",
        );
        let w = t.words(&HashSet::new());
        assert!(w.contains("cach"));
        assert!(w.contains(&stem("widget")));
        // Stop words, inline code, identifiers and fences are no words.
        for gone in [
            "please", "the", "used", "fix", "fixed", "issu", "resolv", "hidden", "hashmap",
            "user_id",
        ] {
            assert!(!w.contains(gone), "{gone} in {w:?}");
        }
        // Inflections end alike, in English and Dutch.
        for (a, z) in [
            ("tokens", "token"),
            ("invalidated", "invalidation"),
            ("entries", "entry"),
            ("ontbreekt", "ontbreken"),
            ("lists", "listed"),
        ] {
            assert_eq!(stem(a), stem(z), "{a} / {z}");
        }
        // A short stop word only goes as written.
        assert!(Text::new("date ops")
            .words(&HashSet::new())
            .contains(&stem("date")));
        assert!(Text::new("a 42 x").words(&HashSet::new()).is_empty());
    }

    /// The thresholds against the cases they were set on: each pair's word
    /// score, and what the rules make of it unmarked (`net`), marked by the
    /// agent (`marked`, medium severity) and marked as a high-severity
    /// finding (`high`). `cargo test -p argus-core calibration --
    /// --nocapture` prints them.
    #[test]
    fn calibration() {
        struct Case {
            name: &'static str,
            note: &'static str,
            title: &'static str,
            body: &'static str,
            /// The thread's hunk and the finding's line text, when on code.
            code: Option<(&'static str, &'static str)>,
            /// The finding's line (the thread is on new 10).
            line: u32,
            /// A thread on the merge request as a whole.
            general: bool,
            expect: (bool, bool, bool),
        }
        let case = |name, note, title, body, expect| Case {
            name,
            note,
            title,
            body,
            code: None,
            line: 10,
            general: false,
            expect,
        };
        let mut cases = vec![
            case(
                "near duplicate",
                EMPTY_LIST,
                "Indexing an empty items list panics",
                "`items[0]` panics when `items` is empty. Guard with `is_empty()` or use `first()`.",
                (true, true, true),
            ),
            case(
                "paraphrase (EN)",
                "Lookups by id are linear in the widget count; a HashMap keyed by id avoids the scan.",
                "Finding a widget walks the whole Vec",
                "Every call iterates all widgets to match the id; index them in a map instead.",
                (false, false, false),
            ),
            case(
                "close paraphrase (EN)",
                "Lookups by id are linear in the widget count; a HashMap keyed by id avoids the scan.",
                "Linear lookup by widget id",
                "Each lookup scans the Vec; key a HashMap by the widget id.",
                (true, true, true),
            ),
            Case {
                line: 40,
                ..case(
                    "close paraphrase, far away",
                    "Lookups by id are linear in the widget count; a HashMap keyed by id avoids the scan.",
                    "Lookups by widget id are linear",
                    "Every lookup scans the widgets; a HashMap keyed by id avoids the scan.",
                    (false, true, false),
                )
            },
            case(
                "Dutch comment, Dutch finding",
                "De cache wordt nooit geïnvalideerd na een schrijfactie, dus lezers zien verouderde widgets tot de TTL verloopt.",
                "Cache wordt niet geïnvalideerd na schrijven",
                "Na een schrijfactie blijven verouderde widgets in de cache staan tot de TTL verloopt.",
                (true, true, true),
            ),
            case(
                "Dutch comment, English finding",
                "Dit crasht als `items` leeg is: `items[0]` heeft geen check. Gebruik eerst `is_empty()`.",
                "Indexing an empty items list panics",
                "`items[0]` panics when `items` is empty; guard with `is_empty()` first.",
                (false, false, false),
            ),
            case(
                "probe: naming nit vs auth",
                "Rename `user_id` to `owner_id` in `load_widget`.",
                "Missing authorization check in load_widget",
                "`load_widget` returns any widget by `user_id` without checking the caller owns it; compare `owner_id` with the session user.",
                (false, false, false),
            ),
            case(
                "probe: suggestion vs TTL",
                "```suggestion\nlet ttl = Duration::from_secs(300);\n```",
                "TTL of 0 disables the cache",
                "With `ttl = 0` every lookup misses; default to `Duration::from_secs(300)`.",
                (false, false, false),
            ),
            case(
                "probe: filler words",
                "Thanks, fixed. Issues resolved.",
                "Retry loop keeps firing after the first request resolved",
                "The fixed-interval retry issues a new request even after the first one resolved; stop the loop on success.",
                (false, false, false),
            ),
            case(
                "different issue, same line",
                EMPTY_LIST,
                "TTL of 0 disables the cache",
                "With `ttl = 0` every lookup misses, so the cache never serves a hit. Default to 300.",
                (false, false, false),
            ),
            case(
                "new part of a marked thread",
                "Lookups by id are linear in the widget count; a HashMap keyed by id avoids the scan.",
                "The widget map is never cleared on logout",
                "Entries of a logged-out user stay in memory for good; clear them in `logout`.",
                (false, false, false),
            ),
        ];
        for (i, (title, body)) in NEW_PARTS.into_iter().enumerate() {
            cases.push(Case {
                code: Some((LOOKUP_HUNK, LOOKUP_LINE)),
                ..case(
                    ["probe r2: memory", "probe r2: unwrap", "probe r2: tenant"][i],
                    LOOKUP_NOTE,
                    title,
                    body,
                    (false, false, false),
                )
            });
        }
        cases.push(Case {
            general: true,
            ..case(
                "probe r2: tests vs token leak",
                "Please add a test for the token refresh path.",
                "Token is logged in plain text",
                "The token is logged by the debug call below; the refresh path leaks it.",
                (false, false, false),
            )
        });
        cases.push(Case {
            general: true,
            ..case(
                "probe r2: expiry vs race",
                "The cache never expires entries.",
                "Cache entries race on concurrent writes",
                "Two writers to the cache can interleave so entries are lost; the cache needs a lock.",
                // Marked by the agent, two of the title's five words are
                // the note's: set aside when medium, kept when high.
                (false, true, false),
            )
        });
        cases.push(Case {
            code: Some(("@@ -8,3 +8,3 @@\n ctx\n ctx2\n+let first = items[0];", "")),
            line: 80,
            ..case(
                "probe r2: same mistake at new 80",
                EMPTY_LIST,
                "Indexing an empty items list panics",
                "`items[0]` panics when `items` is empty. Guard with `is_empty()` or use `first()`.",
                (false, true, false),
            )
        });
        eprintln!("{:<34} {:>6}  net    marked high", "case", "words");
        for c in &cases {
            let mut st = stored(AUTHOR, &[c.note], !c.general);
            if let Some((hunk, _)) = c.code {
                st = with_hunk(st, hunk);
            }
            let raised = raised_points(&[(&st, None)], &[]);
            let line_text = c.code.map(|(_, l)| l).filter(|l| !l.is_empty());
            let f = finding("src/cache.rs", c.line, c.title, c.body);
            let high = of_severity(f.clone(), Severity::High);
            let words = raised[0].score(&Candidate::finding(&f, line_text));
            let got = (
                raised[0]
                    .judge(&Candidate::finding(&f, line_text), false)
                    .is_some(),
                raised[0]
                    .judge(&Candidate::finding(&f, line_text), true)
                    .is_some(),
                raised[0]
                    .judge(&Candidate::finding(&high, line_text), true)
                    .is_some(),
            );
            eprintln!(
                "{:<34} {:>6.2}  {:<6} {:<6} {}",
                c.name, words, got.0, got.1, got.2
            );
            assert_eq!(got, c.expect, "{}: {words}", c.name);
        }
    }
}
