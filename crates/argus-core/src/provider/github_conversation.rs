//! A GitHub pull request's conversation as threads without a code anchor:
//! its issue comments (`issues/{n}/comments`) and the summary bodies of its
//! reviews (`pulls/{n}/reviews`, the text sent with Approve / Comment /
//! Request changes).
//!
//! GitLab threads MR-level discussions itself; GitHub's conversation is one
//! flat list. Each entry starts its own thread unless it reads as a reply,
//! decided from what came before it only (so a new comment never moves an
//! older one, and thread ids stay put across polls):
//!
//! 0. **An Argus reply**: Argus ends its replies with a marker naming the
//!    thread (an HTML comment on its own last line, invisible on GitHub,
//!    see [`reply_text`]). It is honoured only on the user's own entries
//!    (being invisible, anyone else could file a comment under the user's
//!    thread with it) and only when it names an earlier entry of this PR's
//!    conversation: a marker copied from another PR, or naming nothing
//!    before it, is ignored.
//! 1. **A quote reply** (GitHub's "Quote reply"): an entry opening with a
//!    `>` block whose every line is a line of an earlier entry joins that
//!    entry's thread. Preferred is the latest entry that wrote the lines
//!    itself, else the latest that quoted them. A quoted line is a whole
//!    line, or (a selection quoted) at least [`QUOTE_PART`] characters of
//!    one starting at a word; list bullets are ignored, as GitHub writes a
//!    quoted item back with its own. A quote of one line must be at least
//!    [`QUOTE_MIN`] characters ("> ok", "> +1" match far too much); an
//!    alert (`> [!NOTE]`) and an indented `>` (code) are not quotes.
//! 2. **The PR author answering**: the PR author's first entry after the
//!    user wrote joins the thread the user last wrote in. On GitHub the
//!    author answers a remark with a plain new comment, so this is the
//!    user's thread getting a reply, which round detection counts as "the
//!    author replied" (as a reply in a GitLab MR-level discussion of the
//!    user's is). Only the very next one, wherever it is filed: after an
//!    answer by quote elsewhere, a later "Rebased." answers nothing. Chat
//!    commands (`/retest`) and remarks to someone else (`@dependabot
//!    rebase`, "PTAL @carol") are not answers and do not count; what is in
//!    code (`` `@param` ``, a fenced `@Nullable`) is neither.
//! 3. Anything else (the user's own plain comments, other reviewers', bots')
//!    is a thread of its own, as a GitLab comment outside a thread is.
//!
//! Ids are the entries' own: a thread is keyed by its first entry. What
//! follows from that, and stays so: deleting a thread's first entry re-keys
//! the thread to its next one (or splits it), and editing an entry can
//! re-file the quote replies after it, so a draft, verdict override or
//! expanded state kept under the old key no longer finds its thread. Argus's
//! own replies keep their thread by the marker as long as its first entry
//! exists.

use chrono::{DateTime, Utc};

use super::model::{Note, Thread};

/// Thread and note id prefixes: the anchors GitHub itself uses on a PR page
/// (`#issuecomment-…`, `#pullrequestreview-…`). Review comment threads are
/// bare numbers, so these never collide with them, nor with each other.
const ISSUE_COMMENT: &str = "issuecomment-";
const REVIEW: &str = "pullrequestreview-";

/// The longest quoted line a reply carries (characters).
const QUOTE_MAX: usize = 200;

/// A quoted line at least this long may be part of a line: a selection
/// quoted with GitHub's "Quote reply", or a line [`quote`] cut (older Argus
/// replies have no marker). It matches a line it is in, from a word on.
const QUOTE_PART: usize = 24;

/// The shortest one-line quote that files a reply (characters).
const QUOTE_MIN: usize = 12;

/// The marker Argus ends a conversation reply with: `<!-- argus:reply-to
/// issuecomment-123 -->`.
const MARKER_OPEN: &str = "<!-- argus:reply-to ";
const MARKER_CLOSE: &str = " -->";

/// What [`quote`] puts after an `@` so a quoted mention pings nobody again.
const ZERO_WIDTH_SPACE: char = '\u{200B}';

/// A conversation entry's platform id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ConversationId {
    /// An issue comment on the PR.
    Comment(u64),
    /// A submitted review with a summary body.
    Review(u64),
}

impl ConversationId {
    /// The id of a conversation thread or note; `None` for anything else
    /// (a review comment thread's bare number).
    pub(super) fn parse(id: &str) -> Option<Self> {
        let number = |s: &str| {
            (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                .then(|| s.parse().ok())
                .flatten()
        };
        if let Some(n) = id.strip_prefix(ISSUE_COMMENT) {
            return number(n).map(Self::Comment);
        }
        id.strip_prefix(REVIEW).and_then(number).map(Self::Review)
    }

    pub(super) fn key(self) -> String {
        match self {
            Self::Comment(n) => format!("{ISSUE_COMMENT}{n}"),
            Self::Review(n) => format!("{REVIEW}{n}"),
        }
    }
}

/// One conversation entry.
pub(super) struct Entry {
    pub id: ConversationId,
    pub author: String,
    /// As written: what quotes are matched against.
    pub body: String,
    /// What the note shows: a review's body headed by its verdict.
    pub shown: String,
    pub created_at: DateTime<Utc>,
}

/// The heading a review's summary shows under, by review state; `None` for
/// a review that is not submitted (`PENDING`, visible to its author only).
pub(super) fn review_label(state: &str) -> Option<&'static str> {
    match state {
        "APPROVED" => Some("Approved"),
        "CHANGES_REQUESTED" => Some("Requested changes"),
        "COMMENTED" => Some("Commented"),
        "DISMISSED" => Some("Dismissed"),
        _ => None,
    }
}

/// A review summary as a note body: the verdict as its own paragraph, so
/// a body that opens with a list or heading still renders as one.
pub(super) fn review_note(label: &str, body: &str) -> String {
    format!("**{label}**\n\n{body}")
}

fn same_user(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// An entry placed in a thread, as later quotes are matched against it.
struct Placed {
    id: ConversationId,
    thread: usize,
    /// Its own lines (not quoted), normalized ([`normalize`]).
    own: Vec<String>,
    /// The lines it quotes, every level stripped, normalized.
    quoted: Vec<String>,
}

/// The conversation's threads (see the module docs for the rules), oldest
/// first. `me` is the current user, `pr_author` the PR's author.
pub(super) fn threads(mut entries: Vec<Entry>, me: &str, pr_author: &str) -> Vec<Thread> {
    entries.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    let mut threads: Vec<Thread> = Vec::new();
    let mut placed: Vec<Placed> = Vec::new();
    // The thread the user last wrote in, until the PR author answered there.
    let mut awaiting: Option<usize> = None;
    for e in entries {
        let by_marker = reply_marker(&e.body)
            .filter(|_| same_user(&e.author, me))
            .and_then(|id| placed.iter().find(|p| p.id == id))
            .map(|p| p.thread);
        let by_quote = || {
            let quote = opening_quote(&e.body);
            if !quote_files_a_reply(&quote) {
                return None;
            }
            let has = |lines: &[String], q: &String| lines.iter().any(|l| line_matches(l, q));
            let wrote = |p: &&Placed| quote.iter().all(|q| has(&p.own, q));
            let carries = |p: &&Placed| quote.iter().all(|q| has(&p.own, q) || has(&p.quoted, q));
            placed
                .iter()
                .rev()
                .find(wrote)
                .or_else(|| placed.iter().rev().find(carries))
                .map(|p| p.thread)
        };
        let author_answering = same_user(&e.author, pr_author)
            && !same_user(&e.author, me)
            && answers_the_user(&e.body, me);
        let target = by_marker
            .or_else(by_quote)
            .or(if author_answering { awaiting } else { None });
        let note = Note {
            id: e.id.key(),
            author: e.author.clone(),
            body: without_marker(&e.shown),
            created_at: e.created_at,
            system: false,
        };
        let index = match target {
            Some(i) => {
                threads[i].notes.push(note);
                i
            }
            None => {
                threads.push(Thread {
                    id: e.id.key(),
                    author: e.author.clone(),
                    created_at: e.created_at,
                    resolved: false,
                    outdated: false,
                    anchor: None,
                    notes: vec![note],
                });
                threads.len() - 1
            }
        };
        // The author's next answer, wherever it went, is the one: a later
        // plain comment no longer answers the user.
        if same_user(&e.author, me) {
            awaiting = Some(index);
        } else if author_answering || (same_user(&e.author, pr_author) && awaiting == Some(index)) {
            awaiting = None;
        }
        let (own, quoted) = split_lines(&e.body);
        placed.push(Placed {
            id: e.id,
            thread: index,
            own,
            quoted,
        });
    }
    threads
}

/// A line as quotes are compared: trimmed, without a list bullet (`-`,
/// `*`, `+`, `1.`, `1)`) and without the zero-width spaces [`quote`] puts
/// after an `@`.
fn normalize(line: &str) -> String {
    let line = line.trim();
    let line = without_bullet(line).unwrap_or(line);
    line.chars().filter(|c| *c != ZERO_WIDTH_SPACE).collect()
}

/// `line` after its list bullet and the space behind it; `None` for a line
/// that is no list item.
fn without_bullet(line: &str) -> Option<&str> {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let rest = match digits {
        0 => line.strip_prefix(['-', '*', '+'])?,
        1..=9 => line[digits..].strip_prefix(['.', ')'])?,
        _ => return None,
    };
    rest.strip_prefix([' ', '\t']).map(str::trim_start)
}

/// Whether a quoted line `q` is (from) the line `line`: the whole line, or
/// at least [`QUOTE_PART`] characters of it that start at a word.
fn line_matches(line: &str, q: &str) -> bool {
    if line == q {
        return true;
    }
    if q.chars().count() < QUOTE_PART {
        return false;
    }
    line.match_indices(q).any(|(i, _)| {
        !line[..i]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// The quote text of a block-quote line; `None` for any other line. Up to
/// three spaces may come before the `>`; more (or a tab) is code.
fn quote_text(line: &str) -> Option<&str> {
    let rest = line.trim_start_matches(' ');
    (line.len() - rest.len() < 4)
        .then(|| rest.strip_prefix('>'))
        .flatten()
        .map(|q| q.trim_start_matches(|c: char| c == '>' || c.is_whitespace()))
}

/// A body's own lines and its quoted ones, normalized, blanks dropped.
fn split_lines(body: &str) -> (Vec<String>, Vec<String>) {
    let mut own = Vec::new();
    let mut quoted = Vec::new();
    for line in body.lines() {
        let (list, text) = match quote_text(line) {
            Some(q) => (&mut quoted, normalize(q)),
            None => (&mut own, normalize(line)),
        };
        if !text.is_empty() {
            list.push(text);
        }
    }
    (own, quoted)
}

/// The text of a body's opening `>` block, one entry per non-empty line,
/// every quote level stripped and normalized. Empty for no opening block,
/// and for a GitHub alert (`> [!NOTE]`), which is a callout, not a quote.
fn opening_quote(body: &str) -> Vec<String> {
    let lines: Vec<String> = body
        .lines()
        .skip_while(|l| l.trim().is_empty())
        .map_while(quote_text)
        .map(normalize)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.first().is_some_and(|l| is_alert(l)) {
        return Vec::new();
    }
    lines
}

/// `[!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]`, `[!CAUTION]`.
fn is_alert(line: &str) -> bool {
    line.strip_prefix("[!")
        .and_then(|l| l.strip_suffix(']'))
        .is_some_and(|kind| {
            ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"]
                .iter()
                .any(|k| kind.eq_ignore_ascii_case(k))
        })
}

/// Whether an opening quote is enough to file a reply: two lines or more,
/// or one of [`QUOTE_MIN`] characters.
fn quote_files_a_reply(quote: &[String]) -> bool {
    match quote {
        [] => false,
        [one] => one.chars().count() >= QUOTE_MIN,
        _ => true,
    }
}

/// Whether an entry by the PR author can be an answer to the user: not
/// chat commands only (`/retest`), and not addressed to someone else (it
/// mentions others and not the user: `@dependabot rebase`, "PTAL @carol").
/// Code is neither (`@Nullable`, `` `@types/node` ``): only the prose
/// around it is read, and code alone is an answer.
fn answers_the_user(body: &str, me: &str) -> bool {
    let (prose, has_code) = without_code(body);
    let (own, _) = split_lines(&prose);
    if own.is_empty() {
        return has_code;
    }
    if own.iter().all(|l| l.starts_with('/')) {
        return false;
    }
    let mentions: Vec<&str> = own.iter().flat_map(|l| mentions(l)).collect();
    mentions.is_empty() || mentions.iter().any(|m| same_user(m, me))
}

/// `body` without its fenced code blocks (```` ``` ```` or `~~~`, up to
/// three spaces in; an unclosed one runs to the end) and inline code spans,
/// and whether it had any.
fn without_code(body: &str) -> (String, bool) {
    let mut out = String::with_capacity(body.len());
    let mut had = false;
    // The open fence: its character and length.
    let mut fence: Option<(char, usize)> = None;
    for line in body.lines() {
        let rest = line.trim_start_matches(' ');
        let indent = line.len() - rest.len();
        let run = |c: char| rest.chars().take_while(|x| *x == c).count();
        if let Some((c, n)) = fence {
            if indent < 4 && run(c) >= n && rest.trim_start_matches(c).trim().is_empty() {
                fence = None;
            }
            continue;
        }
        if indent < 4 {
            if let Some(c) = ['`', '~'].into_iter().find(|c| run(*c) >= 3) {
                fence = Some((c, run(c)));
                had = true;
                continue;
            }
        }
        let (text, inline) = without_inline_code(line);
        had |= inline;
        out.push_str(&text);
        out.push('\n');
    }
    (out, had)
}

/// `line` with each inline code span (a run of backticks up to the next
/// run as long) replaced by a space, and whether it had any. A run with no
/// closing one is text.
fn without_inline_code(line: &str) -> (String, bool) {
    let ticks = |s: &str| s.bytes().take_while(|b| *b == b'`').count();
    let mut out = String::with_capacity(line.len());
    let mut had = false;
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        let n = ticks(&rest[start..]);
        let after = &rest[start + n..];
        let mut from = 0;
        let mut close = None;
        while let Some(i) = after[from..].find('`') {
            let at = from + i;
            let m = ticks(&after[at..]);
            if m == n {
                close = Some(at);
                break;
            }
            from = at + m;
        }
        match close {
            Some(at) => {
                out.push_str(&rest[..start]);
                out.push(' ');
                rest = &after[at + n..];
                had = true;
            }
            None => {
                out.push_str(&rest[..start + n]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    (out, had)
}

/// The `@login`s in a line (not an e-mail address: the `@` starts a word).
fn mentions(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut prev: Option<char> = None;
    for (i, c) in line.char_indices() {
        if c == '@' && !prev.is_some_and(|p| p.is_alphanumeric() || p == '_' || p == '-') {
            let rest = &line[i + 1..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .unwrap_or(rest.len());
            if end > 0 {
                out.push(&rest[..end]);
            }
        }
        prev = Some(c);
    }
    out
}

/// The thread a body's marker names: its last non-blank line, exactly.
fn reply_marker(body: &str) -> Option<ConversationId> {
    let last = body.lines().rev().map(str::trim).find(|l| !l.is_empty())?;
    let id = last.strip_prefix(MARKER_OPEN)?.strip_suffix(MARKER_CLOSE)?;
    ConversationId::parse(id)
}

/// A body without its closing marker line, as a note shows it.
fn without_marker(body: &str) -> String {
    if reply_marker(body).is_none() {
        return body.to_string();
    }
    let trimmed = body.trim_end();
    let cut = trimmed.rfind('\n').map_or(0, |i| i + 1);
    trimmed[..cut].trim_end().to_string()
}

/// What Argus posts as a reply to the conversation thread `thread` whose
/// first entry is `root`: the [`quote`] of the root (for GitHub readers),
/// the reply, and the marker that files it under `thread` (rule 0).
pub(super) fn reply_text(thread: ConversationId, root: &str, body: &str) -> String {
    let quote = quote(root).map(|q| format!("{q}\n\n")).unwrap_or_default();
    format!(
        "{quote}{}\n\n{MARKER_OPEN}{}{MARKER_CLOSE}",
        body.trim_end(),
        thread.key()
    )
}

/// The quote a reply to a conversation thread opens with: the first line
/// of the thread's first entry that is not itself a quote and not markup
/// GitHub hides (an HTML comment or tag), cut to [`QUOTE_MAX`] characters,
/// as GitHub's "Quote reply" would. An `@` is followed by a zero-width
/// space so a quoted mention does not ping again. `None` when there is no
/// such line.
fn quote(root: &str) -> Option<String> {
    let visible = without_html_comments(root);
    let line = visible
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && quote_text(l).is_none() && !l.starts_with('<'))
        .map(str::to_string)
        .or_else(|| opening_quote(&visible).into_iter().next())?;
    let line: String = line.chars().take(QUOTE_MAX).collect();
    let line = line
        .trim_end()
        .replace('@', &format!("@{ZERO_WIDTH_SPACE}"));
    Some(format!("> {line}"))
}

/// `text` without its `<!-- … -->` comments (an unclosed one runs to the
/// end, as on GitHub).
fn without_html_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start + 4..].find("-->") {
            Some(end) => rest = &rest[start + 4 + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "rev-fixture";
    const AUTHOR: &str = "alice-fixture";

    fn at(minute: u32) -> DateTime<Utc> {
        format!("2026-10-10T10:{minute:02}:00Z").parse().unwrap()
    }

    fn comment(id: u64, author: &str, minute: u32, body: &str) -> Entry {
        Entry {
            id: ConversationId::Comment(id),
            author: author.into(),
            body: body.into(),
            shown: body.into(),
            created_at: at(minute),
        }
    }

    /// (thread id, note ids) per thread.
    fn shape(threads: &[Thread]) -> Vec<(String, Vec<String>)> {
        threads
            .iter()
            .map(|t| (t.id.clone(), t.notes.iter().map(|n| n.id.clone()).collect()))
            .collect()
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_authors_plain_answer_joins_the_thread_the_user_last_wrote_in() {
        let t = threads(
            vec![
                comment(1, AUTHOR, 0, "Ready for review."),
                comment(2, ME, 1, "Why a constant?"),
                comment(3, "carol-fixture", 2, "CI is green."),
                comment(4, AUTHOR, 3, "Moved it to settings."),
                comment(5, ME, 4, "Thanks. One more: the name?"),
            ],
            ME,
            AUTHOR,
        );
        // The author's comment before the user wrote anything stands alone,
        // and so does a third party's plain comment; the user's plain
        // comments each start a thread.
        assert_eq!(
            shape(&t),
            [
                ("issuecomment-1".into(), ids(&["issuecomment-1"])),
                (
                    "issuecomment-2".into(),
                    ids(&["issuecomment-2", "issuecomment-4"])
                ),
                ("issuecomment-3".into(), ids(&["issuecomment-3"])),
                ("issuecomment-5".into(), ids(&["issuecomment-5"])),
            ]
        );
        assert!(t
            .iter()
            .all(|t| t.anchor.is_none() && !t.resolved && !t.outdated));
    }

    #[test]
    fn a_quote_reply_joins_the_thread_it_quotes_whoever_writes_it() {
        let t = threads(
            vec![
                comment(1, AUTHOR, 0, "Ready for review.\nThe TTL is open."),
                comment(2, ME, 1, "Why a constant?"),
                comment(3, "carol-fixture", 2, "> Why a constant?\n\nAgreed."),
                comment(4, "carol-fixture", 3, "CI is green."),
                // The author quoting someone else: not the user's thread.
                comment(5, AUTHOR, 4, "> CI is green.\n\nThanks!"),
                // The user replying in the author's thread (nested quote
                // and a quote of two lines; whole lines only).
                comment(6, ME, 5, ">> Ready for review.\n> The TTL is open.\n\nOk."),
                // A quote that matches nothing: rule 2 for the author.
                comment(7, AUTHOR, 6, "> something else\n\nSee above."),
            ],
            ME,
            AUTHOR,
        );
        assert_eq!(
            shape(&t),
            [
                (
                    "issuecomment-1".into(),
                    ids(&["issuecomment-1", "issuecomment-6", "issuecomment-7"])
                ),
                (
                    "issuecomment-2".into(),
                    ids(&["issuecomment-2", "issuecomment-3"])
                ),
                (
                    "issuecomment-4".into(),
                    ids(&["issuecomment-4", "issuecomment-5"])
                ),
            ]
        );
    }

    #[test]
    fn review_summaries_take_part_and_order_is_by_time() {
        let review = Entry {
            id: ConversationId::Review(9),
            author: ME.into(),
            body: "Please split the module.".into(),
            shown: review_note("Requested changes", "Please split the module."),
            created_at: at(0),
        };
        // Listed out of order: sorted by time first.
        let t = threads(
            vec![comment(1, AUTHOR, 1, "Split in c4."), review],
            ME,
            AUTHOR,
        );
        assert_eq!(
            shape(&t),
            [(
                "pullrequestreview-9".into(),
                ids(&["pullrequestreview-9", "issuecomment-1"])
            )]
        );
        assert_eq!(
            t[0].notes[0].body,
            "**Requested changes**\n\nPlease split the module."
        );
    }

    #[test]
    fn ids_round_trip_and_reject_everything_else() {
        for id in [ConversationId::Comment(42), ConversationId::Review(7)] {
            assert_eq!(ConversationId::parse(&id.key()), Some(id));
        }
        for bad in [
            "42",
            "issuecomment-",
            "issuecomment-4x",
            "pullrequestreview-../1",
            "issuecomment--1",
            "",
        ] {
            assert_eq!(ConversationId::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn quotes_the_first_own_line() {
        assert_eq!(
            quote("\n  Why a constant?  \nmore").as_deref(),
            Some("> Why a constant?")
        );
        assert_eq!(quote("> earlier\n\nMine.").as_deref(), Some("> Mine."));
        assert_eq!(quote("> only a quote").as_deref(), Some("> only a quote"));
        assert_eq!(quote(" \n"), None);
        let long = "é".repeat(300);
        assert_eq!(quote(&long).unwrap().chars().count(), 2 + QUOTE_MAX);
        // What a reply quotes files it under the quoted thread.
        let root = "Ready for review.\nThe TTL is open.";
        let reply = format!("{}\n\nOk.", quote(root).unwrap());
        let t = threads(
            vec![comment(1, AUTHOR, 0, root), comment(2, ME, 1, &reply)],
            ME,
            AUTHOR,
        );
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].notes.len(), 2);
    }

    #[test]
    fn an_argus_reply_lands_in_its_thread_by_the_marker_not_the_quote() {
        // Two plain comments by others that open the same way: the quote
        // alone would file the reply under the later one.
        let root = "A few things:\n- the TTL";
        let reply = reply_text(ConversationId::Comment(1), root, "Done.");
        let t = threads(
            vec![
                comment(1, "carol-fixture", 0, root),
                comment(2, "dana-fixture", 1, "A few things:\n- naming"),
                comment(3, ME, 2, &reply),
            ],
            ME,
            AUTHOR,
        );
        assert_eq!(
            shape(&t),
            [
                (
                    "issuecomment-1".into(),
                    ids(&["issuecomment-1", "issuecomment-3"])
                ),
                ("issuecomment-2".into(), ids(&["issuecomment-2"])),
            ]
        );
        // The marker is GitHub-invisible (an HTML comment on its own last
        // line) and never shown in Argus.
        assert!(reply.ends_with("\n\n<!-- argus:reply-to issuecomment-1 -->"));
        assert_eq!(t[0].notes[1].body, "> A few things:\n\nDone.");
    }

    #[test]
    fn a_marker_naming_an_entry_not_in_this_pr_is_ignored() {
        // Copied from another PR, or pointing ahead: no such earlier entry.
        for target in [ConversationId::Comment(99), ConversationId::Comment(3)] {
            let reply = reply_text(target, "", "Agreed.");
            let t = threads(
                vec![
                    comment(1, "carol-fixture", 0, "A remark."),
                    comment(2, "dana-fixture", 1, &reply),
                    comment(3, "erin-fixture", 2, "Another."),
                ],
                ME,
                AUTHOR,
            );
            assert_eq!(t.len(), 3, "{target:?}");
        }
    }

    #[test]
    fn quotes_match_whole_lines_and_prefer_the_entry_that_wrote_them() {
        let t = threads(
            vec![
                comment(1, ME, 0, "This looks wrong to me."),
                // Short quotes: "ok" is in "looks", "+1"/"Done" anywhere.
                comment(2, "carol-fixture", 1, "> ok\n\nagreed"),
                comment(3, "carol-fixture", 2, "> Done\n\nyes"),
                // A substring of a line is not the line.
                comment(4, "dana-fixture", 3, "> This looks wrong\n\nHow so?"),
                // Two lines are enough however short.
                comment(5, "carol-fixture", 4, "Short.\nTwo."),
                comment(6, "dana-fixture", 5, "> Short.\n> Two.\n\nOk."),
                // Quoting the user: the later entry only quotes the line,
                // the user wrote it.
                comment(7, "erin-fixture", 6, "> This looks wrong to me.\n\n+1"),
                comment(8, "frank-fixture", 7, "> This looks wrong to me.\n\nWhy?"),
            ],
            ME,
            AUTHOR,
        );
        assert_eq!(
            shape(&t),
            [
                (
                    "issuecomment-1".into(),
                    ids(&["issuecomment-1", "issuecomment-7", "issuecomment-8"])
                ),
                ("issuecomment-2".into(), ids(&["issuecomment-2"])),
                ("issuecomment-3".into(), ids(&["issuecomment-3"])),
                ("issuecomment-4".into(), ids(&["issuecomment-4"])),
                (
                    "issuecomment-5".into(),
                    ids(&["issuecomment-5", "issuecomment-6"])
                ),
            ]
        );
    }

    #[test]
    fn alerts_and_indented_code_are_not_quotes() {
        let t = threads(
            vec![
                comment(1, ME, 0, "Run the tests first:\nnpm test --watch=false"),
                comment(2, "carol-fixture", 1, "> [!NOTE]\n> npm test --watch=false"),
                comment(
                    3,
                    "carol-fixture",
                    2,
                    "    > npm test --watch=false\n\nlike so",
                ),
                comment(4, "carol-fixture", 3, "\t> npm test --watch=false"),
                // Up to three spaces is still a quote.
                comment(5, "dana-fixture", 4, "   > npm test --watch=false\n\nyes"),
            ],
            ME,
            AUTHOR,
        );
        assert_eq!(
            shape(&t),
            [
                (
                    "issuecomment-1".into(),
                    ids(&["issuecomment-1", "issuecomment-5"])
                ),
                ("issuecomment-2".into(), ids(&["issuecomment-2"])),
                ("issuecomment-3".into(), ids(&["issuecomment-3"])),
                ("issuecomment-4".into(), ids(&["issuecomment-4"])),
            ]
        );
    }

    #[test]
    fn only_the_authors_first_real_answer_joins_the_users_thread() {
        let t = threads(
            vec![
                comment(1, ME, 0, "Why a constant?"),
                // Chat-ops and remarks to someone else answer nobody.
                comment(2, AUTHOR, 1, "/retest"),
                comment(3, AUTHOR, 2, "@dependabot rebase"),
                comment(4, AUTHOR, 3, "PTAL @carol-fixture"),
                comment(5, AUTHOR, 4, "Moved it to settings, @rev-fixture."),
                // Already answered: the next one stands alone.
                comment(6, AUTHOR, 5, "Rebased."),
                comment(7, ME, 6, "Thanks. And the name?"),
                comment(8, AUTHOR, 7, "Renamed."),
            ],
            ME,
            AUTHOR,
        );
        assert_eq!(
            shape(&t),
            [
                (
                    "issuecomment-1".into(),
                    ids(&["issuecomment-1", "issuecomment-5"])
                ),
                ("issuecomment-2".into(), ids(&["issuecomment-2"])),
                ("issuecomment-3".into(), ids(&["issuecomment-3"])),
                ("issuecomment-4".into(), ids(&["issuecomment-4"])),
                ("issuecomment-6".into(), ids(&["issuecomment-6"])),
                (
                    "issuecomment-7".into(),
                    ids(&["issuecomment-7", "issuecomment-8"])
                ),
            ]
        );
    }

    #[test]
    fn the_reply_quote_skips_html_and_never_pings() {
        assert_eq!(
            quote("<!--\nPlease fill in\n-->\nReal text").as_deref(),
            Some("> Real text")
        );
        assert_eq!(
            quote("<!-- Reviewable:start -->\n## Overview").as_deref(),
            Some("> ## Overview")
        );
        assert_eq!(quote("<!-- only a template -->"), None);
        assert_eq!(quote("<details>\nx").as_deref(), Some("> x"));
        let q = quote("@carol-fixture can you check?").unwrap();
        assert!(!q.contains("@c"), "{q}");
        // The escaped quote still files a reply without the marker.
        let root = "@carol-fixture can you check?";
        let t = threads(
            vec![
                comment(1, AUTHOR, 0, root),
                comment(2, "carol-fixture", 1, &format!("{q}\n\nChecked.")),
            ],
            ME,
            AUTHOR,
        );
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn a_long_line_cut_by_an_older_argus_reply_still_matches() {
        let root = "x".repeat(500);
        let reply = format!("{}\n\nOk.", quote(&root).unwrap());
        let t = threads(
            vec![comment(1, AUTHOR, 0, &root), comment(2, ME, 1, &reply)],
            ME,
            AUTHOR,
        );
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn a_marker_counts_only_on_the_users_own_entries() {
        // The marker is invisible on GitHub: anyone can file a comment
        // under the user's thread with it. Only the user's (Argus's) own
        // replies are filed by it.
        let t = threads(
            vec![
                comment(1, ME, 0, "Why a constant?"),
                comment(
                    2,
                    "mallory-fixture",
                    1,
                    "unrelated\n\n<!-- argus:reply-to issuecomment-1 -->",
                ),
            ],
            ME,
            AUTHOR,
        );
        assert_eq!(
            shape(&t),
            [
                ("issuecomment-1".into(), ids(&["issuecomment-1"])),
                ("issuecomment-2".into(), ids(&["issuecomment-2"])),
            ]
        );
        // Shown as GitHub shows it: without the hidden line.
        assert_eq!(t[1].notes[0].body, "unrelated");
        // Nor can the PR author answer the user by it.
        let t = threads(
            vec![
                comment(1, "carol-fixture", 0, "CI fails on lint."),
                comment(2, ME, 1, "Why a constant?"),
                comment(
                    3,
                    AUTHOR,
                    2,
                    "Fixed.\n\n<!-- argus:reply-to issuecomment-1 -->",
                ),
            ],
            ME,
            AUTHOR,
        );
        assert_eq!(
            shape(&t),
            [
                ("issuecomment-1".into(), ids(&["issuecomment-1"])),
                (
                    "issuecomment-2".into(),
                    ids(&["issuecomment-2", "issuecomment-3"])
                ),
            ]
        );
    }

    #[test]
    fn the_authors_answer_elsewhere_ends_the_wait_for_one() {
        // The author's first entry after the user is the answer, wherever
        // it is filed: a later "Rebased." does not answer the user.
        let t = threads(
            vec![
                comment(
                    1,
                    "carol-fixture",
                    0,
                    "CI fails on the lint job, please check.",
                ),
                comment(2, ME, 1, "Why a constant?"),
                comment(
                    3,
                    AUTHOR,
                    2,
                    "> CI fails on the lint job, please check.\n\nFixed.",
                ),
                comment(4, AUTHOR, 3, "Rebased on main."),
            ],
            ME,
            AUTHOR,
        );
        assert_eq!(
            shape(&t),
            [
                (
                    "issuecomment-1".into(),
                    ids(&["issuecomment-1", "issuecomment-3"])
                ),
                ("issuecomment-2".into(), ids(&["issuecomment-2"])),
                ("issuecomment-4".into(), ids(&["issuecomment-4"])),
            ]
        );
    }

    #[test]
    fn mentions_and_commands_in_code_are_not_remarks_to_someone_else() {
        for answer in [
            "Yes, annotated it:\n```java\n@Nullable\nString name;\n```",
            "Fixed, see `@types/node` bump.",
            "Documented it with `@param ttl`.",
            "~~~\n@Override\n~~~\nlike that",
            "```\n/retest\n```",
            "Moved to `@carol-fixture`'s helper, @rev-fixture.",
        ] {
            let t = threads(
                vec![
                    comment(1, ME, 0, "Can this be null?"),
                    comment(2, AUTHOR, 1, answer),
                ],
                ME,
                AUTHOR,
            );
            assert_eq!(
                shape(&t),
                [(
                    "issuecomment-1".into(),
                    ids(&["issuecomment-1", "issuecomment-2"])
                )],
                "{answer}"
            );
        }
        // A mention outside the code still addresses someone else.
        for other in ["`x` is fine, PTAL @carol-fixture", "```\nx\n```\n/retest"] {
            let t = threads(
                vec![
                    comment(1, ME, 0, "Can this be null?"),
                    comment(2, AUTHOR, 1, other),
                ],
                ME,
                AUTHOR,
            );
            assert_eq!(t.len(), 2, "{other}");
        }
    }

    #[test]
    fn a_quote_of_part_of_a_line_files_the_reply() {
        // GitHub's "Quote reply" on a selection quotes just that.
        let root = "Should the TTL come from settings instead of a constant? It is hard-coded.";
        for (quote, joins) in [
            ("> Should the TTL come from settings", true),
            ("> the TTL come from settings instead", true),
            // Too short to tell, or cut mid-word.
            ("> the TTL come from", false),
            ("> ould the TTL come from settings instead", false),
        ] {
            let t = threads(
                vec![
                    comment(1, ME, 0, root),
                    comment(2, "carol-fixture", 1, &format!("{quote}\n\nAgreed.")),
                ],
                ME,
                AUTHOR,
            );
            assert_eq!(t.len() == 1, joins, "{quote}");
        }
    }

    #[test]
    fn a_quoted_list_item_matches_whatever_its_bullet() {
        // GitHub's "Quote reply" writes a list item back with its own bullet.
        let t = threads(
            vec![
                comment(
                    1,
                    ME,
                    0,
                    "A few things:\n- the TTL should come from settings\n- naming",
                ),
                comment(
                    2,
                    "carol-fixture",
                    1,
                    "> * the TTL should come from settings\n\nAgreed.",
                ),
            ],
            ME,
            AUTHOR,
        );
        assert_eq!(
            shape(&t),
            [(
                "issuecomment-1".into(),
                ids(&["issuecomment-1", "issuecomment-2"])
            )]
        );
    }

    #[test]
    fn labels_submitted_reviews_only() {
        assert_eq!(review_label("CHANGES_REQUESTED"), Some("Requested changes"));
        assert_eq!(review_label("APPROVED"), Some("Approved"));
        assert_eq!(review_label("PENDING"), None);
    }
}
