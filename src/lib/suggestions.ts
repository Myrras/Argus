/**
 * Suggested changes in comments, as a small diff: the lines a suggestion
 * replaces (removed) and its body (added). Pure (no React, no IPC); the
 * `SuggestedChange` component fetches the file and calls these.
 *
 * - GitHub: ` ```suggestion ` replaces the commented lines, `startLine` to
 *   the anchored line (one line without `startLine`).
 * - GitLab: ` ```suggestion:-N+M ` replaces N lines above to M lines below
 *   the anchored line; a bare ` ```suggestion ` is `-0+0`.
 * - An empty body deletes the lines.
 *
 * The original lines come from the file at the comment's own commit when it
 * can be read, else from GitHub's `diff_hunk` (which ends at the commented
 * line), else they are unavailable and only the added lines show.
 */

import type { Anchor, ProviderId, Side } from "../ipc/types";

/** What a suggestion fence's info string says about its range. */
export type SuggestionSpec =
  /** Bare ` ```suggestion `: the platform's default range. */
  | { kind: "default" }
  /** GitLab ` ```suggestion:-N+M `. */
  | { kind: "relative"; above: number; below: number };

/** 1-based, inclusive, on the anchor's side. */
export interface LineRange {
  start: number;
  end: number;
}

/** The fence's spec, or `null` when the fence is no suggestion. */
export function parseSuggestionInfo(info: string): SuggestionSpec | null {
  const word = info.trim().split(/\s+/)[0] ?? "";
  if (word === "suggestion") return { kind: "default" };
  const m = /^suggestion:-(\d+)\+(\d+)$/.exec(word);
  if (!m) return null;
  return { kind: "relative", above: Number(m[1]), below: Number(m[2]) };
}

/** The anchored line on the anchor's side (`null` for a file-level one). */
function anchoredLine(a: SuggestionAnchor): number | null {
  const line = a.side === "old" ? a.lineOld : (a.lineNew ?? a.lineOld);
  return line !== null && line >= 1 ? line : null;
}

export type SuggestionAnchor = Pick<
  Anchor,
  "side" | "lineOld" | "lineNew" | "startLine"
>;

/** The lines the suggestion replaces; `null` when they cannot be known. */
export function suggestionRange(
  spec: SuggestionSpec,
  anchor: SuggestionAnchor | null,
  provider: ProviderId,
): LineRange | null {
  if (!anchor) return null;
  const line = anchoredLine(anchor);
  if (line === null) return null;
  if (spec.kind === "relative") {
    const start = line - spec.above;
    return start >= 1 ? { start, end: line + spec.below } : null;
  }
  const s = anchor.startLine;
  if (provider === "github" && s !== null && s >= 1 && s <= line) {
    return { start: s, end: line };
  }
  return { start: line, end: line };
}

/** The suggestion's lines; an empty body (delete the lines) is none. */
export function suggestedLines(body: string): string[] {
  const text = body.replace(/\r\n?/g, "\n").replace(/\n$/, "");
  return text === "" ? [] : text.split("\n");
}

/** `range` out of a file's text; `null` when the file is shorter. */
export function linesFromContents(
  contents: string,
  range: LineRange,
): string[] | null {
  const text = contents.replace(/\r\n?/g, "\n");
  const lines = text.split("\n");
  if (text.endsWith("\n")) lines.pop();
  if (range.start < 1 || range.end > lines.length) return null;
  return lines.slice(range.start - 1, range.end);
}

/**
 * `range` out of GitHub's `diff_hunk`, which ends at the commented line:
 * the last lines of the anchor's side. `null` when the range does not end
 * at that line (GitLab's `+M` below), the hunk is too short, or its header
 * disagrees with `anchorLine`.
 */
export function linesFromHunk(
  hunk: string,
  range: LineRange,
  anchorLine: number,
  side: Side,
): string[] | null {
  if (range.end !== anchorLine) return null;
  const rows = hunk.replace(/\r\n?/g, "\n").split("\n");
  const header = rows.find((r) => r.startsWith("@@"));
  const own = rows
    .filter((r) => !r.startsWith("@@") && !r.startsWith("\\"))
    .filter((r) => (side === "old" ? !r.startsWith("+") : !r.startsWith("-")))
    .map((r) => r.slice(1));
  // A trailing newline leaves an empty last row that is no line.
  if (own.length > 0 && own[own.length - 1] === "" && hunk.endsWith("\n")) {
    own.pop();
  }
  if (header) {
    const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(header);
    const first = m ? Number(side === "old" ? m[1] : m[2]) : null;
    if (first !== null && first + own.length - 1 !== anchorLine) return null;
  }
  const n = range.end - range.start + 1;
  if (own.length < n) return null;
  return own.slice(own.length - n);
}

/**
 * Where the original lines can be read: the anchor's side at the comment's
 * commit (head side: its head sha; base side: GitLab's base sha — GitHub
 * sends none). `null` when there is no such commit.
 */
export function originalSource(
  anchor: Pick<Anchor, "side" | "pathOld" | "pathNew" | "headSha" | "baseSha">,
): { sha: string; path: string } | null {
  const sha = anchor.side === "old" ? anchor.baseSha : anchor.headSha;
  const path =
    anchor.side === "old"
      ? (anchor.pathOld ?? anchor.pathNew)
      : (anchor.pathNew ?? anchor.pathOld);
  return sha && path ? { sha, path } : null;
}

/** A suggestion ready to render. */
export interface Suggestion {
  range: LineRange | null;
  /** The replaced lines; `null` when unavailable. */
  removed: string[] | null;
  added: string[];
}

/**
 * The suggestion from what is known without fetching: the range and, for
 * GitHub, the lines from its `diff_hunk`. `contents` (the file at
 * `originalSource`) wins when given and long enough.
 */
export function buildSuggestion(
  spec: SuggestionSpec,
  body: string,
  anchor: (SuggestionAnchor & Pick<Anchor, "diffHunk">) | null,
  provider: ProviderId,
  contents?: string | null,
): Suggestion {
  const added = suggestedLines(body);
  const range = suggestionRange(spec, anchor, provider);
  if (!range || !anchor) return { range, removed: null, added };
  const fromFile = contents != null ? linesFromContents(contents, range) : null;
  const line = anchoredLine(anchor);
  const fromHunk =
    anchor.diffHunk && line !== null
      ? linesFromHunk(anchor.diffHunk, range, line, anchor.side)
      : null;
  return { range, removed: fromFile ?? fromHunk, added };
}
