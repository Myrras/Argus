//! Comments on lines outside a GitHub PR's diff.
//!
//! GitHub takes a review comment only on a line of the diff ("line could
//! not be resolved" otherwise). For unchanged code outside the hunks Argus
//! posts a file comment (`subject_type: "file"`) instead, written so a
//! GitHub reader sees what it is about:
//!
//! ````text
//! [Lines 12–18](https://github.com/o/r/blob/<head>/src/x.rs#L12-L18) of `src/x.rs` (outside the diff):
//!
//! ```rs
//! 12  fn x() {
//! …
//! ```
//!
//! <the comment>
//!
//! <!-- argus:lines <head> 12-18 -->
//! ````
//!
//! The hidden marker on the last line places the thread back at those lines
//! when Argus reads it ([`apply`]); honoured only on the user's own root
//! comment, as the conversation's reply marker is.

use super::model::{same_user, Side, Thread};

const MARKER_OPEN: &str = "<!-- argus:lines ";
const MARKER_CLOSE: &str = " -->";

/// What the header line ends with; also how [`shown`] recognises it.
const HEADER_END: &str = "(outside the diff):";

/// The most lines quoted; the rest is counted.
const EXCERPT_LINES: usize = 15;

/// The longest quoted line (characters).
const EXCERPT_WIDTH: usize = 200;

/// What a marker names: the head the lines were read at, and the lines
/// (new side).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LinesMarker {
    pub sha: String,
    pub start: u32,
    pub end: u32,
}

fn lines_label(start: u32, end: u32) -> String {
    if start == end {
        format!("Line {end}")
    } else {
        format!("Lines {start}–{end}")
    }
}

/// The blob URL of `path` at `sha` with the lines' anchor.
pub(super) fn blob_url(
    host: &str,
    repo: &str,
    path: &str,
    sha: &str,
    start: u32,
    end: u32,
) -> String {
    let anchor = if start == end {
        format!("L{end}")
    } else {
        format!("L{start}-L{end}")
    };
    format!(
        "https://{host}/{repo}/blob/{sha}/{}#{anchor}",
        super::encode_path(path)
    )
}

/// The fence for `lines`: one backtick longer than any run in them, at
/// least three.
fn fence(lines: &[String]) -> String {
    let longest = lines
        .iter()
        .flat_map(|l| l.split(|c| c != '`').map(str::len))
        .max()
        .unwrap_or(0);
    "`".repeat((longest + 1).max(3))
}

/// The fence's info string: the path's extension when it is a plain word
/// (GitHub's highlighter knows most as aliases).
fn language(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty()
                && !ext.is_empty()
                && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            ext
        }
        _ => "",
    }
}

/// The file comment's text: header with link, the excerpt (when read),
/// the user's comment, the marker.
#[allow(clippy::too_many_arguments)]
pub(super) fn comment_text(
    host: &str,
    repo: &str,
    path: &str,
    sha: &str,
    start: u32,
    end: u32,
    excerpt: Option<&[String]>,
    body: &str,
) -> String {
    let url = blob_url(host, repo, path, sha, start, end);
    let shown_path = if path.contains('`') {
        path.to_string()
    } else {
        format!("`{path}`")
    };
    let mut out = format!(
        "[{}]({url}) of {shown_path} {HEADER_END}\n\n",
        lines_label(start, end)
    );
    if let Some(lines) = excerpt.filter(|l| !l.is_empty()) {
        let shown: Vec<String> = lines
            .iter()
            .take(EXCERPT_LINES)
            .map(|l| {
                let l = l.trim_end();
                let cut: String = l.chars().take(EXCERPT_WIDTH).collect();
                if cut.len() < l.len() {
                    cut + "…"
                } else {
                    cut
                }
            })
            .collect();
        let fence = fence(&shown);
        let width = end.to_string().len();
        out.push_str(&fence);
        out.push_str(language(path));
        out.push('\n');
        for (i, l) in shown.iter().enumerate() {
            let n = start as usize + i;
            out.push_str(format!("{n:>width$}  {l}").trim_end());
            out.push('\n');
        }
        let total = (end - start + 1) as usize;
        if total > shown.len() {
            out.push_str(&format!("⋮ {} more lines\n", total - shown.len()));
        }
        out.push_str(&fence);
        out.push_str("\n\n");
    }
    out.push_str(body.trim_end());
    out.push_str(&format!(
        "\n\n{MARKER_OPEN}{sha} {}{MARKER_CLOSE}",
        if start == end {
            end.to_string()
        } else {
            format!("{start}-{end}")
        }
    ));
    out
}

/// The marker on `body`'s last non-blank line, exactly.
pub(super) fn marker(body: &str) -> Option<LinesMarker> {
    let last = body.lines().rev().map(str::trim).find(|l| !l.is_empty())?;
    let inner = last.strip_prefix(MARKER_OPEN)?.strip_suffix(MARKER_CLOSE)?;
    let (sha, lines) = inner.split_once(' ')?;
    if !(7..=64).contains(&sha.len()) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let (start, end) = match lines.split_once('-') {
        Some((a, b)) => (a.parse().ok()?, b.parse().ok()?),
        None => {
            let n = lines.parse().ok()?;
            (n, n)
        }
    };
    (start >= 1 && start <= end).then(|| LinesMarker {
        sha: sha.to_string(),
        start,
        end,
    })
}

/// What Argus shows of a marked comment: the user's own text, without the
/// header, the excerpt and the marker (the thread sits at those lines).
/// A body that does not have that shape loses only its marker.
pub(super) fn shown(body: &str) -> String {
    let trimmed = body.trim_end();
    let without_marker = match trimmed.rfind('\n') {
        Some(i) if marker(&trimmed[i + 1..]).is_some() => trimmed[..i].trim_end(),
        None if marker(trimmed).is_some() => "",
        _ => trimmed,
    };
    let mut lines = without_marker.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    if !(first.starts_with("[Line") && first.ends_with(HEADER_END)) {
        return without_marker.to_string();
    }
    let mut rest: Vec<&str> = lines.skip_while(|l| l.trim().is_empty()).collect();
    if let Some(open) = rest.first().map(|l| l.trim_start()) {
        let fence: String = open.chars().take_while(|c| *c == '`').collect();
        if fence.len() >= 3 {
            if let Some(close) = rest.iter().skip(1).position(|l| l.trim() == fence) {
                rest = rest.split_off(close + 2);
            }
        }
    }
    rest.join("\n").trim().to_string()
}

/// Place the user's marked file comments back at their lines: the anchor
/// gets the marker's lines on the new side at its head and `outside_diff`;
/// the root note shows the user's text. Outdated once the PR's head is no
/// longer the marker's (never shown on lines that may hold other code;
/// the then/now history follows them). `head` is read at most once, and
/// only when a marked thread exists; `None` (unreadable) counts as moved.
pub(super) fn apply(threads: &mut [Thread], me: &str, head: impl FnOnce() -> Option<String>) {
    let mut head = Some(head);
    let mut current: Option<Option<String>> = None;
    for t in threads.iter_mut() {
        if !same_user(&t.author, me) {
            continue;
        }
        let Some(anchor) = t.anchor.as_mut() else {
            continue;
        };
        // Only a file comment: a line comment already has its place.
        if anchor.line_new.is_some() || anchor.line_old.is_some() {
            continue;
        }
        let Some(root) = t.notes.first_mut() else {
            continue;
        };
        if !same_user(&root.author, me) {
            continue;
        }
        let Some(m) = marker(&root.body) else {
            continue;
        };
        let now = current
            .get_or_insert_with(|| head.take().and_then(|f| f()))
            .clone();
        root.body = shown(&root.body);
        anchor.side = Side::New;
        anchor.line_new = Some(m.end);
        anchor.line_old = None;
        anchor.start_line = (m.start < m.end).then_some(m.start);
        anchor.head_sha = m.sha.clone();
        anchor.current_line = None;
        anchor.outside_diff = true;
        t.outdated = now.as_deref() != Some(m.sha.as_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "c0ffee0000000000000000000000000000000001";

    #[test]
    fn writes_a_quote_with_link_and_marker() {
        let lines = vec!["fn x() {".to_string(), "    1".to_string()];
        let text = comment_text(
            "github.com",
            "acme-fixture/app",
            "src/my lib.rs",
            SHA,
            12,
            13,
            Some(&lines),
            "Why?\n",
        );
        assert_eq!(
            text,
            format!(
                "[Lines 12–13](https://github.com/acme-fixture/app/blob/{SHA}/src/my%20lib.rs#L12-L13) of `src/my lib.rs` (outside the diff):\n\n```rs\n12  fn x() {{\n13      1\n```\n\nWhy?\n\n<!-- argus:lines {SHA} 12-13 -->"
            )
        );
        assert_eq!(
            marker(&text),
            Some(LinesMarker {
                sha: SHA.into(),
                start: 12,
                end: 13
            })
        );
        assert_eq!(shown(&text), "Why?");
    }

    #[test]
    fn a_single_line_without_excerpt_and_a_long_range() {
        let text = comment_text("ghe.example", "o/r", "a.rs", SHA, 7, 7, None, "Hm");
        assert!(text.starts_with(&format!(
            "[Line 7](https://ghe.example/o/r/blob/{SHA}/a.rs#L7) of `a.rs`"
        )));
        assert!(text.ends_with(&format!("<!-- argus:lines {SHA} 7 -->")));
        assert_eq!(shown(&text), "Hm");
        let many: Vec<String> = (1..=20).map(|n| format!("l{n} ```")).collect();
        let text = comment_text(
            "github.com",
            "o/r",
            "Makefile",
            SHA,
            1,
            20,
            Some(&many),
            "x",
        );
        // A longer fence than the backticks inside, no language for a
        // name without extension, and the rest counted.
        assert!(text.contains("\n````\n 1  l1 ```\n"));
        assert!(text.contains("⋮ 5 more lines\n````\n\nx\n"));
        assert_eq!(shown(&text), "x");
    }

    #[test]
    fn reads_only_exact_markers() {
        assert_eq!(marker("x\n<!-- argus:lines abc 1-2 -->"), None);
        assert_eq!(marker(&format!("<!-- argus:lines {SHA} 3-2 -->")), None);
        assert_eq!(marker(&format!("<!-- argus:lines {SHA} 0 -->")), None);
        assert_eq!(
            marker(&format!("<!-- argus:lines {SHA} 2-3 -->\n\ntrailing")),
            None
        );
        // A body without Argus' header keeps everything but the marker.
        assert_eq!(
            shown(&format!("Plain\n\n<!-- argus:lines {SHA} 4 -->")),
            "Plain"
        );
    }
}
