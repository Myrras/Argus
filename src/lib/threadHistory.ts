/**
 * Pure rules for showing a thread's history (`ThreadHistory`, M3): the two
 * then/now columns from the backend's diff rows, the column labels, the
 * status line per `LineStatus`, and the timeline's wording. No React.
 *
 * The rows are aligned: a line only one side has leaves a `pad` line on the
 * other, so the two columns stay level side by side.
 */

import type {
  CodeSnapshot,
  LineChange,
  ThreadHistory,
  TimelineEntry,
} from "../ipc/types";

/** `del`/`add`: changed on that side; `pad`: the other side has a line here. */
export type ColumnKind = "same" | "del" | "add" | "pad";

export interface ColumnLine {
  /** The file line number; `null` on a pad line. */
  line: number | null;
  text: string;
  kind: ColumnKind;
  /** Inside the commented range. */
  anchored: boolean;
}

export interface ThenNowColumns {
  /** `null`: no code for that side (unavailable, gone or not located). */
  then: ColumnLine[] | null;
  now: ColumnLine[] | null;
}

const PAD: ColumnLine = { line: null, text: "", kind: "pad", anchored: false };

/** The short form commit ids take everywhere in Argus. */
export function shortSha(sha: string): string {
  return sha.slice(0, 8);
}

function fromSnapshot(s: CodeSnapshot): ColumnLine[] {
  return s.lines.map((text, i) => {
    const line = s.firstLine + i;
    const anchored =
      s.anchorStart !== null &&
      s.anchorEnd !== null &&
      line >= s.anchorStart &&
      line <= s.anchorEnd;
    return { line, text, kind: "same", anchored };
  });
}

/**
 * The then and now columns. An unlocated thread never gets a now column:
 * Argus does not guess a line.
 */
export function thenNowColumns(h: ThreadHistory): ThenNowColumns {
  if (h.status.kind === "notCode") return { then: null, now: null };
  const located = h.status.kind !== "unlocated";
  if (located && h.diffLines.length > 0) {
    return {
      then: h.diffLines.map((r) =>
        r.then
          ? {
              line: r.then.line,
              text: r.then.text,
              kind: r.kind === "same" ? "same" : "del",
              anchored: r.then.anchored,
            }
          : PAD,
      ),
      now: h.diffLines.map((r) =>
        r.now
          ? {
              line: r.now.line,
              text: r.now.text,
              kind: r.kind === "same" ? "same" : "add",
              anchored: r.now.anchored,
            }
          : PAD,
      ),
    };
  }
  // Gone now: the commented lines are what was removed.
  const gone = h.status.kind === "deleted" || h.status.kind === "fileGone";
  const then = h.thenSnapshot ? fromSnapshot(h.thenSnapshot) : null;
  return {
    then: gone
      ? (then?.map((l) => (l.anchored ? { ...l, kind: "del" as const } : l)) ??
        null)
      : then,
    now: located && h.nowSnapshot ? fromSnapshot(h.nowSnapshot) : null,
  };
}

/** `12` or `12–14`. */
function range(start: number | null, end: number | null): string | null {
  if (end === null) return start === null ? null : String(start);
  return start === null || start === end ? String(end) : `${start}–${end}`;
}

/** The then column's caption: `path:line` at the comment's commit. */
export function thenLocation(h: ThreadHistory): string | null {
  const s = h.thenSnapshot;
  if (!s) return null;
  const r = range(s.anchorStart, s.anchorEnd);
  return r ? `${s.path}:${r}` : s.path;
}

/** The now column's caption; `null` when the lines are nowhere now. */
export function nowLocation(h: ThreadHistory): string | null {
  const s = h.status;
  if (s.kind === "located") return `${s.path}:${range(s.startLine, s.line)}`;
  if (s.kind === "deleted") return s.path;
  return null;
}

/** A commit's title from the timeline, when it is there. */
function commitTitle(h: ThreadHistory, sha: string): string | null {
  for (const e of h.timeline) {
    if (e.kind === "commit" && e.sha === sha) return e.title || null;
  }
  return null;
}

function inCommit(h: ThreadHistory, sha: string | null): string {
  if (!sha) return "";
  const title = commitTitle(h, sha);
  return ` in ${shortSha(sha)}${title ? ` ${title}` : ""}`;
}

export type StatusTone = "neutral" | "accent" | "warning";

export interface StatusLine {
  tone: StatusTone;
  text: string;
}

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

/**
 * What happened to the commented code, in one line; `null` for a thread
 * without code. `startedByMe` picks "your comment" over "the comment" (the
 * root the code is tracked from; in a thread the user only replied in it
 * is someone else's).
 */
export function statusLine(
  h: ThreadHistory,
  startedByMe: boolean,
): StatusLine | null {
  const s = h.status;
  switch (s.kind) {
    case "notCode":
      return null;
    case "unlocated":
      return {
        tone: "warning",
        text: `Could not locate the lines now: ${s.reason}`,
      };
    case "deleted":
      return {
        tone: "warning",
        text: `The commented lines were removed${inCommit(h, s.removedBy)}`,
      };
    case "fileGone":
      return {
        tone: "warning",
        text: `The file was deleted${inCommit(h, s.removedBy)}`,
      };
    case "located": {
      const then = h.thenSnapshot;
      const thenLines = then ? range(then.anchorStart, then.anchorEnd) : null;
      const nowLines = range(s.startLine, s.line);
      const parts: string[] = [];
      if (s.renamed) {
        parts.push(
          then && then.path !== s.path
            ? `Renamed ${then.path} → ${s.path}`
            : `Renamed to ${s.path}`,
        );
      }
      if (s.moved && thenLines !== null && thenLines !== nowLines) {
        parts.push(`moved from L${thenLines} to L${nowLines}`);
      }
      const n = h.touchedBy.length;
      parts.push(
        s.touched
          ? n > 0
            ? `touched by ${plural(n, "commit")}`
            : "touched since the comment"
          : `untouched since ${startedByMe ? "your" : "the"} comment`,
      );
      const text = parts.join(" · ");
      return {
        tone: s.touched ? "accent" : "neutral",
        text: text.charAt(0).toUpperCase() + text.slice(1),
      };
    }
  }
}

/** What a commit did to the commented lines, for its timeline row. */
export function changeText(c: LineChange): string {
  switch (c.kind) {
    case "modified":
      return "changed the lines";
    case "deleted":
      return "removed the lines";
    case "fileGone":
      return "deleted the file";
    case "renamed":
      return `renamed ${c.from} → ${c.to}`;
  }
}

/** Long enough to fold behind "Show more" in the timeline. */
export function isLongBody(body: string): boolean {
  return body.length > 700 || body.split("\n").length > 12;
}

/** A stable React key for a timeline entry. */
export function entryKey(e: TimelineEntry, i: number): string {
  switch (e.kind) {
    case "note":
      return `note-${e.noteId}`;
    case "commit":
      return `commit-${e.sha}`;
    default:
      return `${e.kind}-${i}`;
  }
}
