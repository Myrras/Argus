/**
 * Find in the review (Cmd+F): what the find bar searches and in which
 * order. Pure (no React, no stores, no CodeMirror): `state/findStore` feeds
 * it the review's threads, files, findings and the files' contents, and runs
 * it in slices so a large review stays responsive.
 *
 * The page reads top to bottom, as `lib/focusModel.pageStops` walks it:
 * the Conversation's threads (unresolved, comments, resolved; then "On
 * other files"), then each shown file — what is listed at its top
 * (outdated threads, AI findings without a line, threads without a usable
 * line), then the diff by line, each line's threads and findings under it.
 * The diff is both sides: every head line, and the removed lines of the
 * base (a changed chunk's base lines, which the unified view shows as a
 * block above the chunk). Unchanged base lines are the head's lines again,
 * so they count once. Collapsed unchanged stretches count: the search reads
 * the contents, not the editor.
 *
 * A thread is its header (the author: kind `thread`) and its notes (kind
 * `note`, the comment's visible text: `lib/markdownText`, no HTML or hidden
 * markers); a finding its title and body (kind `finding`). Each match
 * names its unit (`unit`) and its index there (`ordinal`), which is how the
 * highlighting picks the current one out.
 */

import type {
  FileEntry,
  Finding,
  FindingId,
  Side,
  StoredThread,
  ThreadId,
} from "../ipc/types";
import {
  conversationThreads,
  groupedOrder,
  placeThreads,
  threadsOnFile,
} from "./diffThreads";
import { CONVERSATION_CARD } from "./focusModel";
import { lineMatches, textMatches } from "./findQuery";
import { markdownText } from "./markdownText";

export type FindScope = "everything" | "code" | "comments";

export type FindKind = "code" | "thread" | "note" | "finding";

export interface FindMatch {
  kind: FindKind;
  /** The card it shows in: a file's path or `CONVERSATION_CARD`. */
  card: string;
  /** The file (code, or a thread or finding on a file). */
  file?: string;
  /** Code: the side of the line. */
  side?: Side;
  /** Code: the line (1-based) on that side. */
  line?: number;
  threadId?: ThreadId;
  noteId?: string;
  findingId?: FindingId;
  /** The highlighted unit: a code side, a note, a thread header, a finding part. */
  unit: string;
  /** The match's index in its unit (code: in its line). */
  ordinal: number;
  /** Offsets in the line (code) or in the unit's text. */
  start: number;
  end: number;
  /** Its place inside the card (see `rowOf…`); orders the matches. */
  row: number;
}

// -- Units ---------------------------------------------------------------------

export function codeUnit(path: string, side: Side): string {
  return `code:${side}:${path}`;
}
export function threadUnit(threadId: ThreadId): string {
  return `thread:${threadId}`;
}
export function noteUnit(threadId: ThreadId, noteId: string): string {
  return `note:${threadId}:${noteId}`;
}
export function findingUnit(id: FindingId, part: "title" | "body"): string {
  return `finding:${part}:${id}`;
}

/** A match's identity across re-searches (the current one stays current). */
export function matchKey(m: FindMatch): string {
  return `${m.unit}|${m.line ?? ""}|${m.start}`;
}

// -- Files ---------------------------------------------------------------------

/** A changed chunk as `@codemirror/merge`'s `Chunk` has it (offsets). */
export interface ChunkSpan {
  fromA: number;
  toA: number;
  fromB: number;
  toB: number;
}

/** A run of removed base lines and the head line its block sits above. */
export interface RemovedRun {
  /** 1-based, inclusive. */
  from: number;
  to: number;
  /** The head line the block shows above (one past the end: at the end). */
  before: number;
}

/** A changed chunk in lines, for mapping a base line onto the head. */
interface LineChunk {
  /** Its first base line (the line at `fromA`). */
  aStart: number;
  /** Its last base line; below `aStart` for a pure insertion. */
  aEnd: number;
  /** The first lines after it on each side. */
  nextA: number;
  nextB: number;
  /** The head line its base-side items show under (unified view). */
  under: number;
}

/** A file's two sides, ready to search (`prepareCode`). */
export interface PreparedCode {
  oldLines: string[];
  newLines: string[];
  removed: RemovedRun[];
  chunks: LineChunk[];
}

function lines(text: string): string[] {
  return text === "" ? [] : text.split("\n");
}

function lineStarts(list: readonly string[]): number[] {
  const out: number[] = [];
  let at = 0;
  for (const l of list) {
    out.push(at);
    at += l.length + 1;
  }
  return out;
}

/**
 * The 1-based line holding offset `pos` (`starts`: each line's start);
 * past the text's end, the line after the last.
 */
function lineAt(starts: readonly number[], textLength: number, pos: number) {
  if (starts.length === 0) return 1;
  if (pos > textLength) return starts.length + 1;
  let lo = 0;
  let hi = starts.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (starts[mid] <= pos) lo = mid;
    else hi = mid - 1;
  }
  return lo + 1;
}

/**
 * A file's sides and its changed chunks (as the diff editor computes them,
 * `Chunk.build`) as lines: which base lines were removed, and where their
 * block shows on the head.
 */
export function prepareCode(
  original: string,
  modified: string,
  chunks: readonly ChunkSpan[],
): PreparedCode {
  const oldLines = lines(original);
  const newLines = lines(modified);
  const startsA = lineStarts(oldLines);
  const startsB = lineStarts(newLines);
  const atA = (pos: number) => lineAt(startsA, original.length, pos);
  const atB = (pos: number) => lineAt(startsB, modified.length, pos);
  const removed: RemovedRun[] = [];
  const out: LineChunk[] = [];
  for (const c of chunks) {
    const hasA = c.toA > c.fromA && oldLines.length > 0;
    const aStart = atA(c.fromA);
    const aEnd = hasA ? atA(Math.max(c.fromA, c.toA - 1)) : aStart - 1;
    // As `mapOldLine`: the chunk's last head line (its first, when none).
    const under = Math.min(
      Math.max(1, newLines.length),
      atB(Math.max(c.fromB, Math.min(modified.length, c.toB - 1))),
    );
    out.push({ aStart, aEnd, nextA: atA(c.toA), nextB: atB(c.toB), under });
    if (!hasA) continue;
    // The block sits above the chunk's first head line, or after the last
    // line when the chunk starts past it.
    const before = newLines.length === 0 ? 1 : atB(c.fromB);
    removed.push({ from: aStart, to: aEnd, before });
  }
  return { oldLines, newLines, removed, chunks: out };
}

/** The head line a base-side line's items show under (`mapOldLine`). */
function underLine(code: PreparedCode, line: number): number {
  let startA = 1;
  let startB = 1;
  for (const c of code.chunks) {
    if (line < c.aStart) break;
    if (line <= c.aEnd) return c.under;
    startA = c.nextA;
    startB = c.nextB;
  }
  return Math.max(1, Math.min(code.newLines.length, startB + (line - startA)));
}

/**
 * Why a file's code is not searched, or null: what its card does not show
 * as a diff either (binary, a large diff waiting for "Load anyway", no
 * patch). `waitsForLoadAnyway` is the card's own rule.
 */
export function skipReason(
  f: Pick<
    FileEntry,
    "binary" | "tooLarge" | "patch" | "additions" | "deletions"
  >,
  waitsForLoadAnyway: boolean,
): string | null {
  if (f.binary) return "binary file";
  if (f.tooLarge) return "the platform sent no diff (too large)";
  if (waitsForLoadAnyway) {
    return `large diff (${f.additions + f.deletions} lines)`;
  }
  if (f.patch === null) return "no patch from the platform";
  return null;
}

// -- Search ----------------------------------------------------------------------

/** A finding as the page shows it: its title and its body (null: hidden). */
export interface FindingText {
  finding: Finding;
  title: string;
  body: string | null;
}

/** What the page shows, as `pageStops` takes it, plus the loaded code. */
export interface ReviewPage {
  threads: readonly StoredThread[];
  /** The whole (unfiltered) file list; null while loading or failed. */
  files: readonly FileEntry[] | null;
  failed: boolean;
  /** `CONVERSATION_CARD`, then the shown files' paths. */
  order: readonly string[];
  since: boolean;
  /** The findings shown (AI pass on; dismissed ones only when shown). */
  findings: readonly FindingText[];
}

interface Entry {
  row: number;
  matches: FindMatch[];
}

function textEntry(
  base: Omit<FindMatch, "ordinal" | "start" | "end" | "unit">,
  unit: string,
  text: string,
  re: RegExp,
  plain: boolean,
): FindMatch[] {
  const found = textMatches(plain ? text : markdownText(text), re);
  return found.map(([start, end], ordinal) => ({
    ...base,
    unit,
    ordinal,
    start,
    end,
  }));
}

/** A thread's matches: its header, then its notes in order. */
function threadMatches(
  t: StoredThread,
  card: string,
  row: number,
  re: RegExp,
): FindMatch[] {
  const file = card === CONVERSATION_CARD ? undefined : card;
  const base = {
    kind: "thread" as const,
    card,
    file,
    threadId: t.threadId,
    row,
  };
  const out = textEntry(base, threadUnit(t.threadId), t.author, re, true);
  for (const n of t.notes) {
    out.push(
      ...textEntry(
        { ...base, kind: "note", noteId: n.id },
        noteUnit(t.threadId, n.id),
        n.body,
        re,
        false,
      ),
    );
  }
  return out;
}

function findingMatches(f: FindingText, row: number, re: RegExp): FindMatch[] {
  const base = {
    kind: "finding" as const,
    card: f.finding.path,
    file: f.finding.path,
    findingId: f.finding.id,
    row,
  };
  const out = textEntry(
    base,
    findingUnit(f.finding.id, "title"),
    f.title,
    re,
    true,
  );
  if (f.body !== null) {
    out.push(
      ...textEntry(base, findingUnit(f.finding.id, "body"), f.body, re, false),
    );
  }
  return out;
}

/** The Conversation card's matches, in its order. */
export function searchConversation(page: ReviewPage, re: RegExp): FindMatch[] {
  const talk = conversationThreads(page.threads, page.files, page.failed);
  const list = [
    ...groupedOrder(talk.conversation),
    ...groupedOrder(talk.elsewhere),
  ];
  return list.flatMap((t, i) => threadMatches(t, CONVERSATION_CARD, i, re));
}

/** Rows inside a file card: what is listed at its top comes first. */
const ROW_OUTDATED = -3;
const ROW_LISTED_FINDING = -2;
const ROW_LISTED_THREAD = -1;
/** Items sit under their line; a removed run's block above its line. */
const UNDER = 0.3;
const ABOVE = -0.3;

/**
 * One file card's matches in page order: its listed threads and findings,
 * then the code and the items under its lines. `code` is null until its
 * contents are in (only the threads and findings then), and for a file
 * whose code is not searched. `scope` leaves out the code or the rest.
 */
export function searchFile(
  page: ReviewPage,
  file: FileEntry,
  code: PreparedCode | null,
  re: RegExp,
  scope: FindScope,
): FindMatch[] {
  const path = file.path;
  const entries: Entry[] = [];
  if (scope !== "code") {
    const placed = placeThreads(threadsOnFile(page.threads, file), {
      since: page.since,
    });
    for (const t of placed.outdated) {
      entries.push({
        row: ROW_OUTDATED,
        matches: threadMatches(t, path, ROW_OUTDATED, re),
      });
    }
    const count = {
      a: code?.oldLines.length ?? Infinity,
      b: code?.newLines.length ?? Infinity,
    };
    const listed: StoredThread[] = [...placed.unplaced];
    const inline: Entry[] = [];
    for (const p of placed.inline) {
      if (p.line > count[p.side]) {
        listed.push(p.thread);
        continue;
      }
      const at = p.side === "a" && code ? underLine(code, p.line) : p.line;
      const row = at + UNDER;
      inline.push({ row, matches: threadMatches(p.thread, path, row, re) });
    }
    for (const f of page.findings) {
      if (f.finding.path !== path) continue;
      const side = f.finding.side === "old" ? "a" : "b";
      const line = f.finding.line;
      const fits =
        line >= 1 && line <= count[side] && !(side === "a" && page.since);
      if (!fits) {
        entries.push({
          row: ROW_LISTED_FINDING,
          matches: findingMatches(f, ROW_LISTED_FINDING, re),
        });
        continue;
      }
      const at = side === "a" && code ? underLine(code, line) : line;
      // After the threads of the same line.
      const row = at + UNDER + 0.01;
      inline.push({ row, matches: findingMatches(f, row, re) });
    }
    for (const t of listed) {
      entries.push({
        row: ROW_LISTED_THREAD,
        matches: threadMatches(t, path, ROW_LISTED_THREAD, re),
      });
    }
    entries.push(...inline);
  }
  if (code && scope !== "comments") {
    const unitNew = codeUnit(path, "new");
    code.newLines.forEach((text, i) => {
      const found = lineMatches(text, re);
      if (found.length === 0) return;
      const line = i + 1;
      entries.push({
        row: line,
        matches: found.map(([start, end], ordinal) => ({
          kind: "code",
          card: path,
          file: path,
          side: "new",
          line,
          unit: unitNew,
          ordinal,
          start,
          end,
          row: line,
        })),
      });
    });
    const unitOld = codeUnit(path, "old");
    for (const r of code.removed) {
      const row = r.before + ABOVE;
      for (let line = r.from; line <= r.to; line++) {
        const found = lineMatches(code.oldLines[line - 1] ?? "", re);
        if (found.length === 0) continue;
        entries.push({
          row,
          matches: found.map(([start, end], ordinal) => ({
            kind: "code",
            card: path,
            file: path,
            side: "old",
            line,
            unit: unitOld,
            ordinal,
            start,
            end,
            row,
          })),
        });
      }
    }
  }
  // Stable: entries of one row keep the order they were added in.
  entries.sort((x, y) => x.row - y.row);
  return entries.flatMap((e) => e.matches);
}

/** Every match of the page, in page order (`code` per path, when loaded). */
export function searchReview(
  page: ReviewPage,
  code: ReadonlyMap<string, PreparedCode>,
  re: RegExp,
  scope: FindScope,
): FindMatch[] {
  const byPath = new Map((page.files ?? []).map((f) => [f.path, f]));
  const out: FindMatch[] = [];
  for (const card of page.order) {
    if (card === CONVERSATION_CARD) {
      if (scope !== "code") out.push(...searchConversation(page, re));
      continue;
    }
    const f = byPath.get(card);
    if (f) out.push(...searchFile(page, f, code.get(card) ?? null, re, scope));
  }
  return out;
}

// -- Position ----------------------------------------------------------------------

/** A place on the page: a card (its index in `order`) and a row in it. */
export interface PagePlace {
  card: number;
  row: number;
}

/**
 * The first match at or after `place` (the reading position), else the
 * first one (wrapping); -1 without matches.
 */
export function firstFrom(
  matches: readonly FindMatch[],
  order: readonly string[],
  place: PagePlace | null,
): number {
  if (matches.length === 0) return -1;
  if (!place) return 0;
  const index = new Map(order.map((c, i) => [c, i]));
  const at = matches.findIndex((m) => {
    const card = index.get(m.card) ?? -1;
    return card > place.card || (card === place.card && m.row >= place.row);
  });
  return at < 0 ? 0 : at;
}

/** "3 of 27", "No results", or what the search is doing. */
export function findStatus(s: {
  re: RegExp | null;
  error: string | null;
  matches: readonly unknown[];
  current: number;
  capped: boolean;
  searching: boolean;
  progress: { done: number; total: number };
}): { text: string; searching: string | null } {
  if (s.error) return { text: s.error, searching: null };
  if (!s.re) return { text: "", searching: null };
  const loading =
    s.searching && s.progress.total > 0 && s.progress.done < s.progress.total
      ? `Searching ${s.progress.done} of ${s.progress.total} files…`
      : null;
  const n = s.matches.length;
  const total = `${n}${s.capped ? "+" : ""}`;
  if (n === 0) {
    return {
      text: s.searching ? "" : "No results",
      searching: loading ?? (s.searching ? "Searching…" : null),
    };
  }
  return {
    text: s.current >= 0 ? `${s.current + 1} of ${total}` : `${total} found`,
    searching: loading,
  };
}
