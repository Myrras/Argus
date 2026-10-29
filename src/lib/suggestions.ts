/**
 * Suggested changes in comments: reading them as a small diff (the lines a
 * suggestion replaces, removed, and its body, added) and writing them
 * (the block "Insert suggestion" puts in a composer, when one can be
 * written, and whether a block still covers the comment's lines). Pure (no
 * React, no IPC); `SuggestedChange` and `useComposerSuggestion` fetch the
 * file and call these.
 *
 * - GitHub: ` ```suggestion ` replaces the commented lines, `startLine` to
 *   the anchored line (one line without `startLine`). A reply in a review
 *   thread is a comment of that thread, on its lines.
 * - GitLab: ` ```suggestion:-N+M ` replaces N lines above to M lines below
 *   the anchored line, the note's `new_line` (the last line of a
 *   multi-line comment's `line_range`; a reply shares its discussion's
 *   position); either part may be left out (`Gitlab::Diff::
 *   SuggestionsParser`: `-1`, `+1`, `-1+2`), a bare ` ```suggestion ` is
 *   `-0+0`. GitLab's own button writes `-N+0` with N = selected lines − 1,
 *   and takes at most 100 lines above.
 * - Neither takes one on a removed line (GitLab: `suggestible?` = not
 *   removed; GitHub only offers it on the head side).
 * - A body holding ``` fences gets a longer fence, as GitLab's
 *   `repeatCodeBackticks` does: one more backtick than its longest run.
 * - An empty body deletes the lines.
 *
 * The original lines come from the file at the comment's own commit when it
 * can be read, else from GitHub's `diff_hunk` (which ends at the commented
 * line), else they are unavailable and only the added lines show.
 */

import type {
  Anchor,
  FileEntry,
  OutsideDiff,
  ProviderId,
  Side,
  StoredThread,
} from "../ipc/types";

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
  // GitLab's SUGGESTION_CONTEXT: either part may be left out.
  const m = /^suggestion:(?:-(\d+))?(?:\+(\d+))?$/.exec(word);
  if (!m) return null;
  return {
    kind: "relative",
    above: Number(m[1] ?? 0),
    below: Number(m[2] ?? 0),
  };
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

// -- Writing --------------------------------------------------------------

/** GitLab: `SuggestionsParser` takes at most 100 lines above the note's. */
export const GITLAB_MAX_ABOVE = 100;

/** The fence for `lines`: one backtick more than their longest run of 3+. */
export function suggestionFence(lines: string[]): string {
  let longest = 0;
  for (const line of lines) {
    for (const run of line.match(/`{3,}/g) ?? []) {
      longest = Math.max(longest, run.length);
    }
  }
  return "`".repeat(Math.max(2, longest) + 1);
}

/** The info string for `range`: GitHub's is implied by the comment. */
export function suggestionInfo(provider: ProviderId, range: LineRange): string {
  return provider === "gitlab"
    ? `suggestion:-${range.end - range.start}+0`
    : "suggestion";
}

/** A text change: replace `start..end` with `text`, then select `select`. */
export interface TextEdit {
  start: number;
  end: number;
  text: string;
  /** The selection afterwards, in the new text. */
  select: [number, number];
}

/**
 * Insert a suggestion block for `range` holding `lines` at `at` (the
 * caret; a selection stays as it is, the block goes after it). The block
 * gets lines of its own; its content ends up selected, ready to edit.
 */
export function insertSuggestion(
  body: string,
  at: number,
  provider: ProviderId,
  range: LineRange,
  lines: string[],
): TextEdit {
  const pos = Math.max(0, Math.min(at, body.length));
  const fence = suggestionFence(lines);
  const before = pos > 0 && body[pos - 1] !== "\n" ? "\n" : "";
  const after = pos < body.length && body[pos] !== "\n" ? "\n" : "";
  const open = `${before}${fence}${suggestionInfo(provider, range)}\n`;
  const content = lines.join("\n");
  const text = `${open}${content}\n${fence}${after}`;
  const from = pos + open.length;
  return { start: pos, end: pos, text, select: [from, from + content.length] };
}

/** A fenced block of a markdown text (CommonMark's rules, top level). */
export interface Fence {
  /** The block's first character (the opening line's start). */
  start: number;
  /** After the closing fence line (or the text's end when unclosed). */
  end: number;
  /** The info string's first word, where it sits. */
  infoStart: number;
  infoEnd: number;
  info: string;
  /** Its suggestion spec; null for other code. */
  spec: SuggestionSpec | null;
  /** The content: from the line after the opening fence ... */
  contentStart: number;
  /** ... to the start of the closing fence line (or the text's end). */
  contentEnd: number;
}

/**
 * The fenced blocks of `body`, in order. Fences do not nest: inside one,
 * every line is content until a closing fence of the same character at
 * least as long (an unclosed one runs to the end, as CommonMark has it).
 * Blocks inside quotes and lists are not looked for.
 */
export function findFences(body: string): Fence[] {
  const out: Fence[] = [];
  let open: {
    char: string;
    len: number;
    start: number;
    infoStart: number;
    infoEnd: number;
    info: string;
    contentStart: number;
  } | null = null;
  let pos = 0;
  while (pos <= body.length) {
    const nl = body.indexOf("\n", pos);
    const lineEnd = nl === -1 ? body.length : nl;
    const next = nl === -1 ? body.length + 1 : nl + 1;
    const line = body.slice(pos, lineEnd).replace(/\r$/, "");
    if (open) {
      const close = /^ {0,3}(`{3,}|~{3,})[ \t]*$/.exec(line);
      if (close && close[1]![0] === open.char && close[1]!.length >= open.len) {
        out.push(fenceOf(open, pos, Math.min(next, body.length)));
        open = null;
      }
    } else {
      const m = /^( {0,3})(`{3,}|~{3,})(.*)$/.exec(line);
      // A backtick fence's info string has no backticks.
      if (m && !(m[2]![0] === "`" && m[3]!.includes("`"))) {
        const lead = m[3]!.length - m[3]!.trimStart().length;
        const word = m[3]!.trim().split(/\s+/)[0] ?? "";
        const infoStart = pos + m[1]!.length + m[2]!.length + lead;
        open = {
          char: m[2]![0]!,
          len: m[2]!.length,
          start: pos,
          infoStart,
          infoEnd: infoStart + word.length,
          info: m[3]!.trim(),
          contentStart: Math.min(next, body.length),
        };
      }
    }
    if (nl === -1) break;
    pos = next;
  }
  if (open) out.push(fenceOf(open, body.length, body.length));
  return out;
}

function fenceOf(
  o: {
    start: number;
    infoStart: number;
    infoEnd: number;
    info: string;
    contentStart: number;
  },
  contentEnd: number,
  end: number,
): Fence {
  return {
    start: o.start,
    end,
    infoStart: o.infoStart,
    infoEnd: o.infoEnd,
    info: o.info,
    spec: parseSuggestionInfo(o.info),
    contentStart: o.contentStart,
    contentEnd: Math.max(o.contentStart, contentEnd),
  };
}

/** The suggestion blocks of `body`. */
export function suggestionFences(body: string): Fence[] {
  return findFences(body).filter((f) => f.spec !== null);
}

/**
 * The suggestion block whose content holds `from..to` (a caret or a
 * selection): from its first content character to the end of its last
 * content line, so the closing fence's line is outside.
 */
export function suggestionAt(
  body: string,
  from: number,
  to: number = from,
): Fence | null {
  for (const f of suggestionFences(body)) {
    const closed = f.contentEnd < f.end;
    // Closed, the newline before the closing fence ends the last content
    // line; an empty closed block has no content to type in.
    if (closed && f.contentEnd === f.contentStart) continue;
    const last = closed ? f.contentEnd - 1 : f.contentEnd;
    if (from >= f.contentStart && to <= last) return f;
  }
  return null;
}

/** The lines `spec` replaces around a comment on `comment` (head side). */
export function coveredRange(
  spec: SuggestionSpec,
  provider: ProviderId,
  comment: LineRange,
  insertedFor: LineRange | null,
): LineRange {
  if (spec.kind === "relative") {
    return {
      start: comment.end - spec.above,
      end: comment.end + spec.below,
    };
  }
  // GitHub's bare fence is the comment's own range: it can only drift from
  // what the block was written for when the range changed since.
  if (provider === "github") return insertedFor ?? comment;
  return { start: comment.end, end: comment.end };
}

const sameRange = (a: LineRange, b: LineRange) =>
  a.start === b.start && a.end === b.end;

const lineCount = (r: LineRange) => r.end - r.start + 1;

const linesText = (n: number) => `${n} ${n === 1 ? "line" : "lines"}`;

export const rangeText = (r: LineRange) =>
  r.start === r.end ? `line ${r.end}` : `lines ${r.start}–${r.end}`;

/** A suggestion block that does not cover the comment's lines. */
export interface RangeMismatch {
  covered: LineRange;
  comment: LineRange;
  message: string;
}

/**
 * The first suggestion block of `body` whose lines are not the comment's:
 * a GitLab block whose offsets say other lines (edited, or the comment's
 * range changed after inserting it), or a GitHub block written for a range
 * the comment no longer has (`insertedFor`). GitHub takes no offsets, so a
 * `-N+M` fence there is no suggestion of its, and is left alone.
 */
export function suggestionMismatch(
  body: string,
  provider: ProviderId,
  comment: LineRange,
  insertedFor: LineRange | null = null,
): RangeMismatch | null {
  for (const f of suggestionFences(body)) {
    if (provider === "github" && f.spec!.kind === "relative") continue;
    const covered = coveredRange(f.spec!, provider, comment, insertedFor);
    if (sameRange(covered, comment)) continue;
    const a = lineCount(covered);
    const b = lineCount(comment);
    const message =
      a !== b
        ? `The suggestion covers ${linesText(a)} but the comment is on ${b}`
        : `The suggestion covers ${rangeText(covered)} but the comment is on ${rangeText(comment)}`;
    return { covered, comment, message };
  }
  return null;
}

/**
 * Every GitLab suggestion fence of `body` set to cover `range` (`-N+0`
 * relative to its last line), the rest of each info string kept.
 */
export function retargetSuggestions(body: string, range: LineRange): string {
  let out = body;
  // From the end, so earlier offsets stay valid.
  for (const f of suggestionFences(body).reverse()) {
    out =
      out.slice(0, f.infoStart) +
      suggestionInfo("gitlab", range) +
      out.slice(f.infoEnd);
  }
  return out;
}

/**
 * The indentation step of a file: a tab when most indented lines start
 * with one, else the most common step between two lines' leading spaces
 * (2, 3, 4 or 8), else 4 spaces.
 */
export function indentUnit(contents: string): string {
  const lines = contents.replace(/\r\n?/g, "\n").split("\n");
  let tabs = 0;
  let spaced = 0;
  const steps = new Map<number, number>();
  let prev = 0;
  for (const line of lines) {
    if (line.trim() === "") continue;
    if (line.startsWith("\t")) {
      tabs += 1;
      continue;
    }
    const n = line.length - line.replace(/^ +/, "").length;
    if (n > 0) spaced += 1;
    const step = Math.abs(n - prev);
    if ([2, 3, 4, 8].includes(step))
      steps.set(step, (steps.get(step) ?? 0) + 1);
    prev = n;
  }
  if (tabs > 0 && tabs >= spaced) return "\t";
  let best = 4;
  let most = 0;
  for (const [step, count] of steps) {
    if (count > most || (count === most && step < best)) {
      best = step;
      most = count;
    }
  }
  return " ".repeat(best);
}

/**
 * Enter inside a suggestion's content: a new line indented like the one
 * the caret is on (its leading whitespace up to the caret). Null outside
 * one, so Enter keeps its normal behaviour.
 */
export function enterInSuggestion(
  body: string,
  from: number,
  to: number,
): TextEdit | null {
  if (!suggestionAt(body, from, to)) return null;
  const lineStart = body.lastIndexOf("\n", from - 1) + 1;
  const lead = /^[ \t]*/.exec(body.slice(lineStart, from))![0];
  const text = `\n${lead}`;
  const caret = from + text.length;
  return { start: from, end: to, text, select: [caret, caret] };
}

/**
 * Tab (or Shift+Tab, `outdent`) inside a suggestion's content: a caret or
 * a selection within one line gets `unit`; a selection over several lines
 * indents (or outdents) each of them. Shift+Tab on a caret outdents its
 * line. Null outside a suggestion, so Tab keeps its normal behaviour.
 */
export function tabInSuggestion(
  body: string,
  from: number,
  to: number,
  unit: string,
  outdent = false,
): TextEdit | null {
  if (!suggestionAt(body, from, to)) return null;
  const multi = body.slice(from, to).includes("\n");
  if (!multi && !outdent) {
    const caret = from + unit.length;
    return { start: from, end: to, text: unit, select: [caret, caret] };
  }
  const start = body.lastIndexOf("\n", from - 1) + 1;
  // A selection ending at a line's start leaves that line alone.
  const endAt = multi && to > from && body[to - 1] === "\n" ? to - 1 : to;
  const nl = body.indexOf("\n", endAt);
  const end = nl === -1 ? body.length : nl;
  const lines = body.slice(start, end).split("\n");
  let firstShift = 0;
  let total = 0;
  const changed = lines.map((line, i) => {
    let next: string;
    if (outdent) {
      const strip = line.startsWith("\t")
        ? 1
        : Math.min(
            unit === "\t" ? 4 : unit.length,
            line.length - line.replace(/^ +/, "").length,
          );
      next = line.slice(strip);
    } else {
      next = line === "" ? line : unit + line;
    }
    const delta = next.length - line.length;
    if (i === 0) firstShift = delta;
    total += delta;
    return next;
  });
  const text = changed.join("\n");
  const selStart = Math.max(start, from + firstShift);
  const selEnd = multi ? to + total : Math.max(start, to + firstShift);
  return { start, end, text, select: [selStart, selEnd] };
}

// -- Where a suggestion can be written --------------------------------------

/** Where a composer's suggestion goes, or why it cannot have one. */
export type SuggestionPlan =
  | {
      ok: true;
      /** The file at the review head. */
      path: string;
      sha: string;
      /** The comment's lines at that head, on the new side. */
      range: LineRange;
    }
  | { ok: false; reason: string };

const NEW_CODE =
  "Suggestions apply to the new code: comment on a new-side line to suggest a change";

/** What a file without a usable diff says, or null. */
function fileBlocker(
  file: Pick<FileEntry, "binary" | "tooLarge" | "patch"> | null,
): string | null {
  if (!file) return null;
  if (file.binary) return "A binary file takes no suggestions";
  if (file.tooLarge || file.patch === null) {
    return "The platform sent no diff for this file, so it takes no suggestions";
  }
  return null;
}

function gitlabBlocker(provider: ProviderId, range: LineRange): string | null {
  return provider === "gitlab" && range.end - range.start > GITLAB_MAX_ABOVE
    ? `GitLab takes suggestions of at most ${GITLAB_MAX_ABOVE + 1} lines`
    : null;
}

/** A new comment's suggestion: on new-side lines, inside a diff GitHub knows. */
export function commentSuggestionPlan(c: {
  provider: ProviderId;
  path: string;
  side: Side;
  line: number;
  startLine: number | null;
  headSha: string;
  outside?: OutsideDiff | null;
  file: Pick<FileEntry, "binary" | "tooLarge" | "patch"> | null;
}): SuggestionPlan {
  const no = (reason: string): SuggestionPlan => ({ ok: false, reason });
  if (c.side === "old") return no(NEW_CODE);
  if (c.outside === "fallback") {
    return no(
      "GitHub takes no suggestion outside the diff: this comment goes as a file comment",
    );
  }
  const blocked = fileBlocker(c.file);
  if (blocked) return no(blocked);
  const range = { start: c.startLine ?? c.line, end: c.line };
  const big = gitlabBlocker(c.provider, range);
  if (big) return no(big);
  return { ok: true, path: c.path, sha: c.headSha, range };
}

/**
 * A reply's suggestion applies to its thread's lines: a GitHub review
 * thread's current `start_line..line`, a GitLab diff discussion's
 * `line_range` (its note's `new_line` last). `headSha` is the review's
 * head (GitHub needs it: its threads move with the code to the head's
 * lines; null when unknown).
 */
export function replySuggestionPlan(r: {
  provider: ProviderId;
  thread: Pick<StoredThread, "anchor" | "outdated">;
  headSha: string | null;
  file: Pick<FileEntry, "binary" | "tooLarge" | "patch"> | null;
}): SuggestionPlan {
  const no = (reason: string): SuggestionPlan => ({ ok: false, reason });
  const a = r.thread.anchor;
  if (!a) return no("This thread is on the whole review, not on code lines");
  if (r.thread.outdated) {
    return no(
      "This thread is outdated: its lines changed since it was written",
    );
  }
  if (a.outsideDiff) {
    return no(
      "GitHub takes no suggestion outside the diff: this thread is a file comment",
    );
  }
  if (a.side === "old") return no(NEW_CODE);
  const own = a.lineNew;
  const path = a.pathNew ?? a.pathOld;
  if (own === null || own < 1 || !path) {
    return no("This thread is on the whole file, not on lines");
  }
  const blocked = fileBlocker(r.file);
  if (blocked) return no(blocked);
  // GitLab keeps a current discussion's position at the head (one at an
  // older head is outdated), so its own lines and commit are the head's.
  let end = own;
  let sha = a.headSha;
  if (r.provider === "github") {
    // `lineNew` is the line at the comment's commit; GitHub moves the
    // thread with the code, to `currentLine` at the head.
    if (a.currentLine === null || !r.headSha) {
      return no(
        "This thread is outdated: its lines changed since it was written",
      );
    }
    end = a.currentLine;
    sha = r.headSha;
  }
  const span =
    a.startLine !== null && a.startLine <= own ? own - a.startLine : 0;
  const range = { start: Math.max(1, end - span), end };
  const big = gitlabBlocker(r.provider, range);
  if (big) return no(big);
  return { ok: true, path, sha, range };
}

/**
 * The anchor a composer's preview reads its suggestion against: the
 * planned lines on the new side (`buildSuggestion`, `SuggestedChange`).
 */
export function planAnchor(
  range: LineRange,
): SuggestionAnchor & Pick<Anchor, "diffHunk"> {
  return {
    side: "new",
    lineOld: null,
    lineNew: range.end,
    startLine: range.start < range.end ? range.start : null,
    diffHunk: null,
  };
}
