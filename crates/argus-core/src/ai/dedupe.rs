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
//!    ([`RELATED_SIMILARITY`], without the title's share in a note: a short
//!    title shares its subject with any note on it); a checklist item needs
//!    [`MIN_CHECK_SHARED`] shared words besides. A related point the agent
//!    kept anyway is caught, a new part on the same code is kept. A
//!    high-severity finding is never set aside on the agent's mark alone
//!    (an over-eager or steered mark must not bury it).
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
//! Every finding and item that is kept names the points it relates to
//! ("related to …"): the ones the agent marked, and the ones close to it
//! by the same words ([`Raised::relates`]: near the thread at
//! [`RELATED_SIMILARITY`], elsewhere at [`NEAR_SIMILARITY`]). So a close
//! call the net keeps is never shown as if the review said nothing there.
//!
//! Only prose words count: code says what the line is, not what is wrong
//! with it. Left out are fenced blocks (code, suggestions), inline code
//! spans and identifier-shaped words (`snake_case`, `camelCase`, with
//! digits). The words of the code the point is on (the thread's hunk end
//! and the finding's own line) count only when the two texts also share a
//! word of their own that is not a mere part of a program
//! ([`GENERIC_NOUNS`]): "this unwrap panics" and "unwrap on an unknown user
//! panics" make one point about the line, "validate the email input" and
//! "the email input is logged" two. Words are lower-cased and stemmed
//! (English and Dutch suffixes, repeatedly; a doubled final consonant
//! single), and then English and Dutch stop words and stock review phrases
//! are left out (by stem, so `fixed` goes with `fix`; an agent noun such as
//! `gebruiker` is no inflection of `gebruik`). At least
//! [`MIN_SHARED_WORDS`] words must be shared.
//!
//! Similarity is the best of the Jaccard index of the point against the
//! whole thread, the share of the finding's title found in a note (titles
//! of at least [`MIN_TOKENS`] words: a two-word title wholly in a note is
//! its point), and the share of a note found in the finding. The last one
//! catches a comment whose point a longer finding contains in full, which
//! plain Jaccard dilutes; it counts only for a note of at least
//! [`MIN_NOTE_WORDS`] words whose words the finding's title shares too:
//! three words of a short note ("the cache never expires entries") are its
//! subject, and a finding about something else on that subject mentions
//! them in its body.

use std::collections::{HashMap, HashSet};
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
/// this big. With [`MIN_SHARED_WORDS`] shared, a two-word title counts only
/// when wholly in the note ("Password logged"): it says nothing else.
pub(crate) const MIN_TOKENS: usize = 2;
/// The share of a note found in a finding only counts for notes at least
/// this big (see the module docs).
pub(crate) const MIN_NOTE_WORDS: usize = 5;
/// The agent's mark sets a checklist item aside only with at least this
/// many shared words.
pub(crate) const MIN_CHECK_SHARED: usize = 3;
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
    "others",
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
    "mag",
    "kunnen",
    "maar",
    "met",
    "mij",
    "mijn",
    "moet",
    "mogen",
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
/// `issues`), each with whether the stop word itself lost an `-er`. Short
/// function words only match as written: their stems (`dat`, `op`) are
/// stems of real words (`date`, `ops`).
static STOP_STEMS: LazyLock<HashMap<String, bool>> = LazyLock::new(|| {
    let mut stems = HashMap::new();
    for w in STOP_WORDS.iter().filter(|w| w.chars().count() >= 4) {
        let (stemmed, agent) = stem_parts(w);
        *stems.entry(stemmed).or_insert(false) |= agent;
    }
    stems
});

/// Nouns that name a part of a program, not what is wrong with it: shared
/// alone (`the email address input`), they say two texts are about the
/// same code, not that they make the same point. Compared by stem.
const GENERIC_NOUNS: &[&str] = &[
    "argument",
    "arguments",
    "array",
    "data",
    "element",
    "field",
    "function",
    "input",
    "method",
    "number",
    "object",
    "output",
    "parameter",
    "result",
    "string",
    "type",
    "value",
    "variable",
    // Dutch.
    "functie",
    "gegevens",
    "invoer",
    "methode",
    "resultaat",
    "uitvoer",
    "variabele",
    "veld",
    "waarde",
];

static GENERIC: LazyLock<HashSet<String>> =
    LazyLock::new(|| GENERIC_NOUNS.iter().map(|w| stem(w)).collect());

/// Suffixes stripped, longest first, until none applies (English and Dutch
/// inflections; `tokens`/`token` and `ontbreken`/`ontbreekt` end alike).
const SUFFIXES: &[&str] = &[
    "ations", "ation", "ating", "ated", "ates", "ate", "ments", "ment", "ingen", "heden", "heid",
    "ings", "ing", "edly", "ies", "ied", "ed", "es", "en", "er", "ly", "s", "e", "t", "y",
];

/// A word as compared: lower case, suffixes off, a doubled final consonant
/// single (`logged`/`log`, `dropped`/`drop`), Dutch double vowels single.
pub(crate) fn stem(word: &str) -> String {
    stem_parts(word).0
}

/// [`stem`], and whether an `-er` came off: an agent noun (`gebruiker`,
/// `issuer`, `maker`) is a word of its own, not an inflection of the verb
/// it shares a stem with (`gebruik`, `issue`, `make`).
fn stem_parts(word: &str) -> (String, bool) {
    let mut w = word.to_lowercase();
    let mut agent = false;
    loop {
        let stripped = SUFFIXES.iter().find_map(|suffix| {
            let rest = w.strip_suffix(suffix)?;
            (rest.chars().count() >= 3 && !(*suffix == "s" && rest.ends_with('s')))
                .then(|| (rest.to_string(), *suffix == "er"))
        });
        match stripped {
            Some((rest, er)) => {
                w = rest;
                agent |= er;
            }
            None => break,
        }
    }
    let chars: Vec<char> = w.chars().collect();
    if let [.., a, b] = chars[..] {
        if chars.len() >= 4 && a == b && a.is_alphabetic() && !"aeiouy".contains(a) {
            w.pop();
        }
    }
    for (double, single) in [("ee", "e"), ("aa", "a"), ("oo", "o"), ("uu", "u")] {
        w = w.replace(double, single);
    }
    (w, agent)
}

/// A stop word as written, or an inflection of a longer one: not when only
/// an agent noun's `-er` makes the stems meet (`gebruiker` is not
/// `gebruik`), unless the stop word ends so itself (`others`/`other`).
fn is_stop(lower: &str) -> bool {
    if STOP.contains(lower) {
        return true;
    }
    let (stemmed, agent) = stem_parts(lower);
    STOP_STEMS
        .get(&stemmed)
        .is_some_and(|stop_agent| !agent || *stop_agent)
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

    /// The comparable words of this text: stemmed, without identifiers and
    /// stop words.
    fn words(&self) -> HashSet<String> {
        self.prose
            .iter()
            .filter(|w| !identifier(w))
            .filter_map(|w| {
                let lower = w.to_lowercase();
                (!is_stop(&lower)).then(|| stem(&lower))
            })
            .collect()
    }
}

/// Words of code lines, stemmed: what a point on them is about, not what it
/// says about them.
fn code_words<'a>(lines: impl IntoIterator<Item = &'a str>) -> HashSet<String> {
    lines
        .into_iter()
        .flat_map(|l| words_of(diff_content(l)).map(stem).collect::<Vec<_>>())
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

/// How much a point says what a raised point says (see the module docs).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Overlap {
    /// The best of all three measures: what the net goes by.
    pub net: f64,
    /// Without the share of the title found in a note: what the agent's
    /// mark goes by. A short title shares its subject with any note on it
    /// ("Order query ignores soft-deleted flag" and a note on the order
    /// query's N+1), and the mark already says the subject is the same.
    pub marked: f64,
    /// The words the point and the notes share.
    pub shared: usize,
}

impl Overlap {
    fn max(self, other: Overlap) -> Overlap {
        Overlap {
            net: self.net.max(other.net),
            marked: self.marked.max(other.marked),
            shared: self.shared.max(other.shared),
        }
    }
}

/// How much a point (`title` and `whole`, title included) says what
/// `notes` say (see the module docs), counting only overlaps of at least
/// [`MIN_SHARED_WORDS`] words.
fn similarity(
    title: &HashSet<String>,
    whole: &HashSet<String>,
    notes: &[HashSet<String>],
) -> Overlap {
    let all: HashSet<String> = notes.iter().flatten().cloned().collect();
    let n = shared(whole, &all);
    if n < MIN_SHARED_WORDS {
        return Overlap::default();
    }
    let j = jaccard(whole, &all);
    let mut o = Overlap {
        net: j,
        marked: j,
        shared: n,
    };
    for note in notes {
        if shared(whole, note) < MIN_SHARED_WORDS || shared(title, note) < MIN_SHARED_WORDS {
            continue;
        }
        let note_in = contained(note, whole, MIN_NOTE_WORDS);
        o.marked = o.marked.max(note_in);
        o.net = o.net.max(note_in).max(contained(title, note, MIN_TOKENS));
    }
    o
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
    /// The words of each human note (or of a posted finding, its title and
    /// body).
    notes: Vec<HashSet<String>>,
    /// Words of the code it was left on (its hunk end, its then-code).
    code: HashSet<String>,
}

/// A point the run makes: a finding (with its place and the text of its
/// line) or a checklist item (no place).
pub(crate) struct Candidate<'a> {
    place: Option<(&'a str, Side, u32)>,
    /// A high-severity finding: never set aside on the agent's mark alone.
    high: bool,
    title: HashSet<String>,
    /// Title and body.
    whole: HashSet<String>,
    code: HashSet<String>,
}

impl<'a> Candidate<'a> {
    /// `line_text`: the diff's text at the finding's line, when known.
    pub fn finding(f: &'a RawFinding, line_text: Option<&str>) -> Self {
        let title = Text::new(&f.title).words();
        let whole = Text::new(&f.body).words().union(&title).cloned().collect();
        Candidate {
            place: Some((f.path.as_str(), f.side, f.line)),
            high: f.severity == Severity::High,
            title,
            whole,
            code: code_words(line_text),
        }
    }

    pub fn check(c: &'a RawCheck) -> Self {
        let title = Text::new(&c.text).words();
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
            notes: texts.iter().map(|t| Text::new(t).words()).collect(),
            code,
        }
    }

    /// How much `c` says what this point says, prose words only. Words of
    /// the code either is on count only when the two also share a word of
    /// their own that is no mere part of a program ([`GENERIC_NOUNS`]):
    /// then a line word both use as prose ("this unwrap panics", "unwrap
    /// on an unknown user panics") is part of the point; without one, it is
    /// only the code both are on.
    pub fn overlap(&self, c: &Candidate<'_>) -> Overlap {
        let code: HashSet<String> = self.code.union(&c.code).cloned().collect();
        let prose =
            |s: &HashSet<String>| -> HashSet<String> { s.difference(&code).cloned().collect() };
        let notes: Vec<HashSet<String>> = self.notes.iter().map(prose).collect();
        let strict = similarity(&prose(&c.title), &prose(&c.whole), &notes);
        let own = notes
            .iter()
            .flatten()
            .any(|w| c.whole.contains(w) && !GENERIC.contains(w));
        if code.is_empty() || !own {
            return strict;
        }
        strict.max(similarity(&c.title, &c.whole, &self.notes))
    }

    /// The net's score of `c` (see [`Self::overlap`]).
    pub fn score(&self, c: &Candidate<'_>) -> f64 {
        self.overlap(c).net
    }

    /// Whether `c` repeats this point, and why (`related`: the agent listed
    /// this point's id for it).
    pub fn judge(&self, c: &Candidate<'_>, related: bool) -> Option<(RepeatReason, f64)> {
        let o = self.overlap(c);
        // A checklist item is a short question: two shared words are its
        // subject, not its point.
        let enough = c.place.is_some() || o.shared >= MIN_CHECK_SHARED;
        if related && !c.high && enough && o.marked >= RELATED_SIMILARITY {
            return Some((RepeatReason::AgentMarked, o.marked));
        }
        let words = o.net;
        let similar = match c.place {
            _ if self.places.is_empty() => words >= GLOBAL_SIMILARITY,
            None => words >= GLOBAL_SIMILARITY,
            Some((path, side, line)) => {
                let near = self.near(path, side, line);
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

    /// Whether `c` is about this point without (necessarily) repeating it:
    /// near it and [`RELATED_SIMILARITY`], or anywhere and
    /// [`NEAR_SIMILARITY`]. It then names this point ("related to …"), so a
    /// close call the net keeps is never shown as unrelated.
    pub fn relates(&self, c: &Candidate<'_>) -> bool {
        let words = self.score(c);
        match c.place {
            Some((path, side, line)) if self.near(path, side, line) => words >= RELATED_SIMILARITY,
            _ => words >= NEAR_SIMILARITY,
        }
    }

    fn near(&self, path: &str, side: Side, line: u32) -> bool {
        self.places.iter().any(|p| p.near(path, side, line))
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

/// The points `c` names as related: the ones the agent marked, then the
/// ones Argus finds it about ([`Raised::relates`]).
fn related_points(c: &Candidate<'_>, related: &[String], raised: &[Raised]) -> Vec<RaisedRef> {
    let marked = raised.iter().filter(|r| related.contains(&r.of.id()));
    let found = raised
        .iter()
        .filter(|r| !related.contains(&r.of.id()) && r.relates(c));
    marked.chain(found).map(|r| r.of).collect()
}

/// Mark the findings and checklist items of `run` that repeat one of
/// `raised` (counted in `dropped_duplicates` and `skipped_checklist`), and
/// name on each the points it relates to. `diff` is the diff the agent
/// read: the text of a finding's line is code, not a point.
pub(crate) fn classify(run: &mut RunOutput, raised: &[Raised], diff: &FileDiff) {
    for f in &mut run.findings {
        let c = Candidate::finding(f, finding_line(diff, f));
        let (repeat, related) = (
            best_repeat(&c, &f.related, raised),
            related_points(&c, &f.related, raised),
        );
        (f.repeat, f.related_to) = (repeat, related);
    }
    run.dropped_duplicates = run.findings.iter().filter(|f| f.repeat.is_some()).count() as u32;
    for item in &mut run.checklist {
        let c = Candidate::check(item);
        let (repeat, related) = (
            best_repeat(&c, &item.related, raised),
            related_points(&c, &item.related, raised),
        );
        (item.repeat, item.related_to) = (repeat, related);
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
            related_to: Vec::new(),
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
        // A kept item the agent marked names the thread; so does an
        // unmarked one close to it, and one about something else nothing.
        let thread = vec![RaisedRef::Thread(st.thread_id)];
        let related: Vec<_> = run.checklist.iter().map(|c| c.related_to.clone()).collect();
        assert_eq!(
            related,
            vec![thread.clone(), Vec::new(), thread.clone(), thread]
        );
    }

    #[test]
    fn a_close_call_the_net_keeps_names_the_thread() {
        // Same line, same query, another point: kept, but tagged, so it is
        // never shown as if nothing was said about this code.
        let st = stored(
            AUTHOR,
            &["This runs one query per order inside the loop (N+1). Batch it."],
            true,
        );
        let raised = raised_points(&[(&st, None)], &[]);
        let patch = "@@ -9,3 +9,3 @@\n ctx\n+for o in orders { db.lines(o.id)?; }\n tail";
        let mut run = RunOutput {
            findings: vec![
                finding(
                    "src/cache.rs",
                    10,
                    "Order query ignores soft-deleted flag",
                    "Deleted orders are still returned by this query; filter on `deleted_at IS NULL`.",
                ),
                finding(
                    "src/cache.rs",
                    10,
                    "TTL of 0 disables the cache",
                    "With `ttl = 0` every lookup misses.",
                ),
            ],
            ..RunOutput::default()
        };
        classify(&mut run, &raised, &diff("src/cache.rs", patch));
        assert_eq!(run.findings[0].repeat, None);
        assert_eq!(
            run.findings[0].related_to,
            vec![RaisedRef::Thread(st.thread_id)]
        );
        assert!(run.findings[1].related_to.is_empty());
        assert_eq!(run.dropped_duplicates, 0);
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
        let w = t.words();
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
        assert!(Text::new("date ops").words().contains(&stem("date")));
        assert!(Text::new("a 42 x").words().is_empty());
    }

    /// The thresholds against the cases they were set on: each pair's word
    /// score, and what the rules make of it unmarked (`net`), marked by the
    /// agent (`marked`, medium severity), marked as a high-severity finding
    /// (`high`), and whether the unmarked one, kept, still names the thread
    /// (`tag`, "related to …"). `said`: a person reads the finding as the
    /// note's point; such a finding is never kept untagged, and a new point
    /// is never set aside by the net. `cargo test -p argus-core calibration
    /// -- --nocapture` prints the table.
    /// Whether a person reads a finding as the note's point.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Said {
        Yes,
        /// Yes, in other words or another language: no word to go by, only
        /// the agent's mark names the thread.
        Paraphrase,
        No,
    }

    #[test]
    fn calibration() {
        struct Case {
            name: &'static str,
            note: &'static str,
            title: &'static str,
            body: &'static str,
            /// The thread's hunk (the code it was left on).
            hunk: Option<&'static str>,
            /// The text of the finding's line.
            line_text: Option<&'static str>,
            /// The finding's line (the thread is on new 10).
            line: u32,
            /// A thread on the merge request as a whole.
            general: bool,
            said: Said,
            /// `net`, `marked`, `high`, `tag`.
            expect: (bool, bool, bool, bool),
        }
        let case = |name, note, title, body, said, expect| Case {
            name,
            note,
            title,
            body,
            hunk: None,
            line_text: None,
            line: 10,
            general: false,
            said,
            expect,
        };
        let mut cases = vec![
            case(
                "near duplicate",
                EMPTY_LIST,
                "Indexing an empty items list panics",
                "`items[0]` panics when `items` is empty. Guard with `is_empty()` or use `first()`.",
                Said::Yes,
                (true, true, true, true),
            ),
            case(
                "paraphrase (EN)",
                "Lookups by id are linear in the widget count; a HashMap keyed by id avoids the scan.",
                "Finding a widget walks the whole Vec",
                "Every call iterates all widgets to match the id; index them in a map instead.",
                Said::Paraphrase,
                (false, false, false, false),
            ),
            case(
                "close paraphrase (EN)",
                "Lookups by id are linear in the widget count; a HashMap keyed by id avoids the scan.",
                "Linear lookup by widget id",
                "Each lookup scans the Vec; key a HashMap by the widget id.",
                Said::Yes,
                (true, true, true, true),
            ),
            Case {
                line: 40,
                ..case(
                    "close paraphrase, far away",
                    "Lookups by id are linear in the widget count; a HashMap keyed by id avoids the scan.",
                    "Lookups by widget id are linear",
                    "Every lookup scans the widgets; a HashMap keyed by id avoids the scan.",
                    Said::Yes,
                    (false, true, false, true),
                )
            },
            case(
                "Dutch comment, Dutch finding",
                "De cache wordt nooit geïnvalideerd na een schrijfactie, dus lezers zien verouderde widgets tot de TTL verloopt.",
                "Cache wordt niet geïnvalideerd na schrijven",
                "Na een schrijfactie blijven verouderde widgets in de cache staan tot de TTL verloopt.",
                Said::Yes,
                (true, true, true, true),
            ),
            case(
                "Dutch comment, English finding",
                "Dit crasht als `items` leeg is: `items[0]` heeft geen check. Gebruik eerst `is_empty()`.",
                "Indexing an empty items list panics",
                "`items[0]` panics when `items` is empty; guard with `is_empty()` first.",
                Said::Paraphrase,
                (false, false, false, false),
            ),
            case(
                "probe: naming nit vs auth",
                "Rename `user_id` to `owner_id` in `load_widget`.",
                "Missing authorization check in load_widget",
                "`load_widget` returns any widget by `user_id` without checking the caller owns it; compare `owner_id` with the session user.",
                Said::No,
                (false, false, false, false),
            ),
            case(
                "probe: suggestion vs TTL",
                "```suggestion\nlet ttl = Duration::from_secs(300);\n```",
                "TTL of 0 disables the cache",
                "With `ttl = 0` every lookup misses; default to `Duration::from_secs(300)`.",
                Said::No,
                (false, false, false, false),
            ),
            case(
                "probe: filler words",
                "Thanks, fixed. Issues resolved.",
                "Retry loop keeps firing after the first request resolved",
                "The fixed-interval retry issues a new request even after the first one resolved; stop the loop on success.",
                Said::No,
                (false, false, false, false),
            ),
            case(
                "different issue, same line",
                EMPTY_LIST,
                "TTL of 0 disables the cache",
                "With `ttl = 0` every lookup misses, so the cache never serves a hit. Default to 300.",
                Said::No,
                (false, false, false, false),
            ),
            case(
                "new part of a marked thread",
                "Lookups by id are linear in the widget count; a HashMap keyed by id avoids the scan.",
                "The widget map is never cleared on logout",
                "Entries of a logged-out user stay in memory for good; clear them in `logout`.",
                Said::No,
                (false, false, false, false),
            ),
        ];
        for (i, (title, body)) in NEW_PARTS.into_iter().enumerate() {
            cases.push(Case {
                hunk: Some(LOOKUP_HUNK),
                line_text: Some(LOOKUP_LINE),
                ..case(
                    ["probe r2: memory", "probe r2: unwrap", "probe r2: tenant"][i],
                    LOOKUP_NOTE,
                    title,
                    body,
                    Said::No,
                    (false, false, false, false),
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
                Said::No,
                (false, false, false, false),
            )
        });
        cases.push(Case {
            general: true,
            ..case(
                "probe r2: expiry vs race",
                "The cache never expires entries.",
                "Cache entries race on concurrent writes",
                "Two writers to the cache can interleave so entries are lost; the cache needs a lock.",
                Said::No,
                // Two of the title's five words are the note's subject: no
                // longer enough for the agent's mark.
                (false, false, false, false),
            )
        });
        cases.push(Case {
            hunk: Some("@@ -8,3 +8,3 @@\n ctx\n ctx2\n+let first = items[0];"),
            line: 80,
            ..case(
                "probe r2: same mistake at new 80",
                EMPTY_LIST,
                "Indexing an empty items list panics",
                "`items[0]` panics when `items` is empty. Guard with `is_empty()` or use `first()`.",
                Said::No,
                (false, true, false, true),
            )
        });
        // Loop 2 round 3 probes: on the finding's own line, with its text.
        let on_line = |name, note, title, body, line_text, said, expect| Case {
            line_text: Some(line_text),
            ..case(name, note, title, body, said, expect)
        };
        cases.extend([
            on_line(
                "r3: token logged",
                "This logs the token in plain text; secrets end up in our log files.",
                "Token logged in plain text",
                "The bearer token is written to the info log; redact it.",
                "log::info!(\"token: {}\", token);",
                Said::Yes,
                (true, true, true, true),
            ),
            on_line(
                "r3: error dropped",
                "Errors are dropped silently here.",
                "Error silently dropped",
                "The error from `save` is dropped; propagate it with `?`.",
                "let _ = save(x);",
                Said::Yes,
                (true, true, true, true),
            ),
            Case {
                line: 11,
                ..on_line(
                    "r3: unwrap on None",
                    "Missing null check: `user` can be None here and the unwrap panics.",
                    "Unwrap on optional user panics",
                    "`find_user` returns None for unknown ids; `unwrap()` then panics. Return a 404 instead.",
                    "let user = find_user(id).unwrap();",
                    Said::Yes,
                    (true, true, true, true),
                )
            },
            on_line(
                "r3: unwrap, short",
                "This unwrap will panic when the user is not found.",
                "Panic on unknown user",
                "`unwrap()` panics if no user matches the id; return a 404 instead.",
                "let user = find_user(id).unwrap();",
                Said::Yes,
                (true, true, true, true),
            ),
            on_line(
                "r3: N+1",
                "This runs one query per order inside the loop (N+1). Batch it.",
                "N+1 query in order loop",
                "Each iteration issues a separate query for the order lines; fetch them in one batched query.",
                "for o in orders { db.lines(o.id)?; }",
                Said::Yes,
                (true, true, true, true),
            ),
            on_line(
                "r3: SQL injection",
                "SQL injection: the name is concatenated into the query string.",
                "SQL injection via string concatenation",
                "`name` is concatenated into the SQL string; use a bound parameter.",
                "let q = format!(\"SELECT * FROM t WHERE name = '{}'\", name);",
                Said::Yes,
                (true, true, true, true),
            ),
            on_line(
                "r3: racy balance check",
                "Race condition: two requests can both pass the balance check and overdraw.",
                "Balance check is racy",
                "Concurrent withdrawals can both pass the check before either writes, overdrawing the account. Lock the row.",
                "if account.balance >= amount {",
                Said::Yes,
                (true, true, true, true),
            ),
            on_line(
                "r3: NL authorisation",
                "De gebruiker kan hier facturen van andere klanten opvragen; er ontbreekt een autorisatiecheck.",
                "Autorisatiecheck ontbreekt bij facturen",
                "Een gebruiker kan facturen van andere klanten ophalen omdat de autorisatiecheck ontbreekt.",
                "let factuur = repo.get(id)?;",
                Said::Yes,
                (true, true, true, true),
            ),
            on_line(
                "r3: NL timeout",
                "Er staat geen timeout op deze HTTP-call; bij een trage server blijft de request hangen.",
                "HTTP-call zonder timeout",
                "Zonder timeout blijft de request hangen als de server traag reageert.",
                "let resp = client.get(url).send()?;",
                Said::Yes,
                (true, true, true, true),
            ),
            on_line(
                "r3: NL password logged",
                "Hier wordt het wachtwoord gelogd, dat mag niet.",
                "Wachtwoord wordt gelogd",
                "Het wachtwoord komt in de logging terecht; laat het weg.",
                "log::debug!(\"{:?}\", req);",
                Said::Yes,
                (true, true, true, true),
            ),
            on_line(
                "r3: EN password logged",
                "Password is logged here, not allowed.",
                "Password logged",
                "The password ends up in the debug log; remove it.",
                "debug!(\"{:?}\", req);",
                Said::Yes,
                (true, true, true, true),
            ),
            on_line(
                "r3: API key logged",
                "The API key gets logged here.",
                "API key written to the log",
                "The request's API key is logged at debug level; redact it before logging.",
                "debug!(\"{:?}\", req);",
                Said::Yes,
                (true, true, true, true),
            ),
            on_line(
                "r3: query, other point",
                "This runs one query per order inside the loop (N+1). Batch it.",
                "Order query ignores soft-deleted flag",
                "Deleted orders are still returned by this query; filter on `deleted_at IS NULL`.",
                "for o in orders { db.lines(o.id)?; }",
                Said::No,
                (false, false, false, true),
            ),
            on_line(
                "r3: cache key, short note",
                "Cache key looks wrong.",
                "Cache key collides across tenants",
                "The cache key is the widget id only, so tenants share entries and read each other's widgets.",
                "let key = widget.id;",
                Said::No,
                (false, false, false, true),
            ),
            on_line(
                "r3: NL error message",
                "De foutmelding hier is niet duidelijk voor de gebruiker.",
                "Foutmelding lekt interne details",
                "De foutmelding toont de stacktrace en het SQL-statement aan de klant; log die intern en toon een generieke melding.",
                "return Err(e.to_string());",
                Said::No,
                (false, false, false, false),
            ),
            on_line(
                "r3: backoff vs cap",
                "Retries should use exponential backoff.",
                "Retries are not capped",
                "The retry loop never stops on a permanent error; cap the retries at a maximum count.",
                "loop { if send().is_ok() { break; } }",
                Said::No,
                (false, false, false, false),
            ),
            on_line(
                "r3: validate vs logged",
                "Please validate the email address input.",
                "Email address input is logged",
                "The raw email address input is written to the access log, which is personal data.",
                "let email = form.email;",
                Said::No,
                (false, false, false, false),
            ),
        ]);
        let mut wrong = Vec::new();
        eprintln!(
            "{:<34} {:<10} {:>6}  net    marked high   tag",
            "case", "said", "words"
        );
        for c in &cases {
            let mut st = stored(AUTHOR, &[c.note], !c.general);
            if let Some(hunk) = c.hunk {
                st = with_hunk(st, hunk);
            }
            let raised = raised_points(&[(&st, None)], &[]);
            let f = finding("src/cache.rs", c.line, c.title, c.body);
            let high = of_severity(f.clone(), Severity::High);
            let medium = Candidate::finding(&f, c.line_text);
            let words = raised[0].score(&medium);
            let got = (
                raised[0].judge(&medium, false).is_some(),
                raised[0].judge(&medium, true).is_some(),
                raised[0]
                    .judge(&Candidate::finding(&high, c.line_text), true)
                    .is_some(),
                raised[0].relates(&medium),
            );
            eprintln!(
                "{:<34} {:<10} {:>6.2}  {:<6} {:<6} {:<6} {}",
                c.name,
                format!("{:?}", c.said),
                words,
                got.0,
                got.1,
                got.2,
                got.3
            );
            if got != c.expect {
                wrong.push(format!("{}: {got:?} ({words:.2})", c.name));
            }
            // What the review already says is never shown untagged; a new
            // point is never set aside by the net.
            assert!(c.said != Said::Yes || got.0 || got.3, "{} untagged", c.name);
            assert!(c.said != Said::No || !got.0, "{} set aside", c.name);
        }

        // Checklist items against a comment on the merge request as a whole
        // (the item has no place): (`net`, `marked`, `tag`).
        type Check = (
            &'static str,
            &'static str,
            &'static str,
            Said,
            (bool, bool, bool),
        );
        let checks: [Check; 5] = [
            (
                "check: migration",
                "No migration for old caches: existing entries keep the old format.",
                "Do old cache entries need a migration on upgrade?",
                Said::Yes,
                (true, true, true),
            ),
            (
                "check: cache key, short note",
                "Cache key looks wrong.",
                "Cache key collides across tenants",
                Said::No,
                (false, false, false),
            ),
            (
                "check: query, other point",
                "This runs one query per order inside the loop (N+1). Batch it.",
                "Order query ignores soft-deleted flag",
                Said::No,
                (false, false, false),
            ),
            (
                "check: validate vs logged",
                "Please validate the email address input.",
                "Email address input is logged",
                Said::No,
                (false, false, true),
            ),
            (
                "check: NL password logged",
                "Hier wordt het wachtwoord gelogd, dat mag niet.",
                "Wachtwoord wordt gelogd",
                Said::Yes,
                (true, true, true),
            ),
        ];
        for (name, note, text, said, expect) in checks {
            let st = stored(AUTHOR, &[note], false);
            let raised = raised_points(&[(&st, None)], &[]);
            let item = RawCheck::from(text);
            let c = Candidate::check(&item);
            let words = raised[0].score(&c);
            let got = (
                raised[0].judge(&c, false).is_some(),
                raised[0].judge(&c, true).is_some(),
                raised[0].relates(&c),
            );
            eprintln!(
                "{:<34} {:<10} {:>6.2}  {:<6} {:<6} -      {}",
                name,
                format!("{said:?}"),
                words,
                got.0,
                got.1,
                got.2
            );
            if got != expect {
                wrong.push(format!("{name}: {got:?} ({words:.2})"));
            }
            assert!(said != Said::Yes || got.0 || got.2, "{name} untagged");
            assert!(said != Said::No || !got.0, "{name} set aside");
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    #[test]
    fn stems_undo_doubled_consonants_and_keep_agent_nouns() {
        for (a, z) in [("logged", "log"), ("dropped", "drop"), ("logging", "logs")] {
            assert_eq!(stem(a), stem(z), "{a} / {z}");
        }
        // An agent noun is a word of its own, not the stop word's verb.
        for noun in ["gebruiker", "gebruikers", "issuer", "maker"] {
            assert!(!is_stop(noun), "{noun}");
        }
        // Inflections of stop words still go, also those ending in -er.
        for stop in ["gebruikt", "issues", "makes", "others"] {
            assert!(is_stop(stop), "{stop}");
        }
    }
}
