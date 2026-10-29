//! A response as `gh api -i` and `glab api --include` print it: the status
//! line (`HTTP/2.0 200 OK`), the headers, a blank line, the body. Both CLIs
//! print it for any status (and exit non-zero for a non-2xx), so the status
//! line is the truth, not the exit code.

/// One parsed response.
pub(crate) struct HttpReply {
    pub status: u16,
    /// Lowercased names.
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl HttpReply {
    /// `None` when `out` does not start with a status line: the CLI failed
    /// before any response (not installed, offline, logged out).
    pub fn parse(out: &str) -> Option<Self> {
        let (first, mut rest) = out.split_once('\n')?;
        let first = first.trim_end_matches('\r');
        if !first.starts_with("HTTP/") {
            return None;
        }
        let status = first.split_whitespace().nth(1)?.parse().ok()?;
        let mut headers = Vec::new();
        while !rest.is_empty() {
            let (line, after) = rest.split_once('\n').unwrap_or((rest, ""));
            rest = after;
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
            }
        }
        Some(Self {
            status,
            headers,
            body: rest.to_string(),
        })
    }

    /// The first header `name` (any case).
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_glab_included_output() {
        // As glab 1.124 prints a 304 (exit 1) and a 200 with an empty
        // `X-Next-Page`.
        let r = HttpReply::parse("HTTP/1.1 304 Not Modified\r\nDate: x\r\n\r\n").unwrap();
        assert_eq!(r.status, 304);
        assert_eq!(r.body, "");
        let r = HttpReply::parse("HTTP/2.0 200 OK\nEtag: W/\"a\"\nX-Next-Page: \n\nline1\nline2\n")
            .unwrap();
        assert_eq!(r.header("etag").as_deref(), Some("W/\"a\""));
        assert_eq!(r.header("x-next-page").as_deref(), Some(""));
        assert_eq!(r.body, "line1\nline2\n");
        assert!(HttpReply::parse("glab: 401 Unauthorized (HTTP 401)\n").is_none());
        assert!(HttpReply::parse("").is_none());
    }
}
