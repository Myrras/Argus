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
//! 1. **A quote reply** (GitHub's "Quote reply", and how Argus replies, see
//!    [`quote`]): an entry opening with a `>` block whose every line occurs
//!    in an earlier entry joins that entry's thread (the latest such entry).
//! 2. **The PR author answering**: an entry by the PR author that is not a
//!    quote reply joins the thread the user last wrote in, if the user wrote
//!    in the conversation before. On GitHub the author answers a remark with
//!    a plain new comment, so this is the user's thread getting a reply,
//!    which round detection counts as "the author replied" (as a reply in a
//!    GitLab MR-level discussion of the user's is).
//! 3. Anything else (the user's own plain comments, other reviewers', bots')
//!    is a thread of its own, as a GitLab comment outside a thread is.

use chrono::{DateTime, Utc};

use super::model::{Note, Thread};

/// Thread and note id prefixes: the anchors GitHub itself uses on a PR page
/// (`#issuecomment-…`, `#pullrequestreview-…`). Review comment threads are
/// bare numbers, so these never collide with them, nor with each other.
const ISSUE_COMMENT: &str = "issuecomment-";
const REVIEW: &str = "pullrequestreview-";

/// The longest quoted line a reply carries (characters).
const QUOTE_MAX: usize = 200;

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

/// The conversation's threads (see the module docs for the rules), oldest
/// first. `me` is the current user, `pr_author` the PR's author.
pub(super) fn threads(mut entries: Vec<Entry>, me: &str, pr_author: &str) -> Vec<Thread> {
    entries.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    let mut threads: Vec<Thread> = Vec::new();
    // Every entry placed so far: its body and its thread.
    let mut placed: Vec<(String, usize)> = Vec::new();
    // The thread the user last wrote in.
    let mut mine: Option<usize> = None;
    for e in entries {
        let quoted = quoted_lines(&e.body);
        let by_quote = if quoted.is_empty() {
            None
        } else {
            placed
                .iter()
                .rev()
                .find(|(body, _)| quoted.iter().all(|q| body.contains(q.as_str())))
                .map(|(_, i)| *i)
        };
        let author_answering = same_user(&e.author, pr_author) && !same_user(&e.author, me);
        let target = by_quote.or(if author_answering { mine } else { None });
        let note = Note {
            id: e.id.key(),
            author: e.author.clone(),
            body: e.shown,
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
        if same_user(&e.author, me) {
            mine = Some(index);
        }
        placed.push((e.body, index));
    }
    threads
}

/// The text of a body's opening `>` block, one entry per non-empty line,
/// with every quote level stripped.
fn quoted_lines(body: &str) -> Vec<String> {
    body.lines()
        .map(str::trim)
        .skip_while(|l| l.is_empty())
        .take_while(|l| l.starts_with('>'))
        .map(|l| {
            l.trim_start_matches(|c: char| c == '>' || c.is_whitespace())
                .to_string()
        })
        .filter(|l| !l.is_empty())
        .collect()
}

/// The quote a reply to a conversation thread opens with: the first line
/// of the thread's first entry that is not itself a quote (cut to
/// [`QUOTE_MAX`] characters), as GitHub's "Quote reply" would. It is what
/// files the reply under that thread on the next fetch (rule 1). `None`
/// for an empty body.
pub(super) fn quote(root: &str) -> Option<String> {
    let line = root
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('>'))
        .map(str::to_string)
        .or_else(|| quoted_lines(root).into_iter().next())?;
    let line: String = line.chars().take(QUOTE_MAX).collect();
    Some(format!("> {}", line.trim_end()))
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
                // and a quote of two lines).
                comment(
                    6,
                    ME,
                    5,
                    ">> The TTL\n> Ready for review.\n> The TTL is open.\n\nOk.",
                ),
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
    fn labels_submitted_reviews_only() {
        assert_eq!(review_label("CHANGES_REQUESTED"), Some("Requested changes"));
        assert_eq!(review_label("APPROVED"), Some("Approved"));
        assert_eq!(review_label("PENDING"), None);
    }
}
