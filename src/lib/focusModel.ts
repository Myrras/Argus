/**
 * The review page's focus model (M6): what can hold the keyboard focus on
 * the page, and the order `n`/`p` walk it in. Pure rules, no React, no
 * stores: `state/focusStore` feeds it the review's threads, files and
 * findings.
 *
 * The page reads top to bottom: the Conversation (its threads, then "On
 * other files"), then each shown file — its header, the threads listed at
 * the top of the card (outdated, then those without a usable line), then
 * the threads and AI findings under their lines, by line. That is the stop
 * order; `n`/`p` step through the threads and findings in it, `j`/`k` the
 * Conversation and the files (`codeViewStore.move`).
 *
 * A focused diff line (a click in the code) is a position between stops:
 * `n` goes to the first item at or below it, `p` to the last one above.
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
  placeThreads,
  threadsOnFile,
} from "./diffThreads";

/** The Conversation card's key (`codeViewStore.CONVERSATION`). */
export const CONVERSATION_CARD = "\u0000conversation";

export type FocusItem =
  | { kind: "conversation" }
  | { kind: "file"; path: string }
  | {
      kind: "thread";
      threadId: ThreadId;
      mine: boolean;
      /** The card it shows in: a file's path or `CONVERSATION_CARD`. */
      card: string;
      /** Its line in the diff (0: listed at the top of its card). */
      line: number;
      /** The side of that line; null when listed. */
      side: Side | null;
    }
  | {
      kind: "finding";
      findingId: FindingId;
      card: string;
      /** Its line (0 when it cannot sit under one). */
      line: number;
      side: Side;
    }
  | { kind: "line"; path: string; side: Side; line: number };

export type ItemKind = FocusItem["kind"];

/** The card an item belongs to. */
export function cardOf(item: FocusItem): string {
  switch (item.kind) {
    case "conversation":
      return CONVERSATION_CARD;
    case "file":
    case "line":
      return item.path;
    default:
      return item.card;
  }
}

/** Two items are the same stop. */
export function sameItem(a: FocusItem | null, b: FocusItem | null): boolean {
  if (!a || !b || a.kind !== b.kind) return false;
  switch (a.kind) {
    case "conversation":
      return true;
    case "file":
      return a.path === (b as typeof a).path;
    case "thread":
      return a.threadId === (b as typeof a).threadId;
    case "finding":
      return a.findingId === (b as typeof a).findingId;
    case "line": {
      const o = b as typeof a;
      return a.path === o.path && a.side === o.side && a.line === o.line;
    }
  }
}

/** Threads and findings: what `n`/`p` stop on. */
export function isEntry(item: FocusItem): boolean {
  return item.kind === "thread" || item.kind === "finding";
}

export interface PageInput {
  threads: readonly StoredThread[];
  /** The whole (unfiltered) file list; null while loading or failed. */
  files: readonly FileEntry[] | null;
  /** The file list failed to load. */
  failed: boolean;
  /** The shown stops: `CONVERSATION_CARD`, then the shown files' paths. */
  order: readonly string[];
  /** "Since my last review": base-side lines are not placed. */
  since: boolean;
  /** The findings shown (AI pass on; dismissed ones only when shown). */
  findings: readonly Finding[];
}

/** One thread's focus item in `card` (its line as the card places it). */
export function threadItem(
  t: StoredThread,
  card: string,
  since: boolean,
): Extract<FocusItem, { kind: "thread" }> {
  if (card !== CONVERSATION_CARD) {
    const [p] = placeThreads([t], { since }).inline;
    if (p) {
      return {
        kind: "thread",
        threadId: t.threadId,
        mine: t.mine,
        card,
        line: p.line,
        side: p.side === "a" ? "old" : "new",
      };
    }
  }
  return {
    kind: "thread",
    threadId: t.threadId,
    mine: t.mine,
    card,
    line: 0,
    side: null,
  };
}

export function findingItem(
  f: Finding,
  since: boolean,
): Extract<FocusItem, { kind: "finding" }> {
  const listed = f.line < 1 || (f.side === "old" && since);
  return {
    kind: "finding",
    findingId: f.id,
    card: f.path,
    line: listed ? 0 : f.line,
    side: f.side,
  };
}

/** Sort key inside a card: header, listed entries, then by line. */
function lineOf(item: FocusItem): number {
  switch (item.kind) {
    case "conversation":
    case "file":
      return -1;
    case "line":
      return item.line;
    default:
      return item.line;
  }
}

/** Every stop of the page, in page order. */
export function pageStops(input: PageInput): FocusItem[] {
  const out: FocusItem[] = [];
  const talk = conversationThreads(input.threads, input.files, input.failed);
  const byPath = new Map((input.files ?? []).map((f) => [f.path, f]));
  for (const card of input.order) {
    if (card === CONVERSATION_CARD) {
      out.push({ kind: "conversation" });
      for (const t of [...talk.conversation, ...talk.elsewhere]) {
        out.push(threadItem(t, CONVERSATION_CARD, input.since));
      }
      continue;
    }
    const file = byPath.get(card);
    if (!file) continue;
    out.push({ kind: "file", path: card });
    const placed = placeThreads(threadsOnFile(input.threads, file), {
      since: input.since,
    });
    // Listed at the top: outdated first, as the card shows them.
    for (const t of [...placed.outdated, ...placed.unplaced]) {
      out.push({
        kind: "thread",
        threadId: t.threadId,
        mine: t.mine,
        card,
        line: 0,
        side: null,
      });
    }
    const inline: FocusItem[] = [
      ...input.findings
        .filter((f) => f.path === card)
        .map((f) => findingItem(f, input.since)),
      ...placed.inline.map((p): FocusItem => ({
        kind: "thread",
        threadId: p.thread.threadId,
        mine: p.thread.mine,
        card,
        line: p.line,
        side: p.side === "a" ? "old" : "new",
      })),
    ];
    // By line; on one line threads before findings, the base side first.
    const rank = (i: FocusItem) =>
      (i.kind === "finding" ? 2 : 0) +
      ("side" in i && i.side === "new" ? 1 : 0);
    inline.sort((x, y) => lineOf(x) - lineOf(y) || rank(x) - rank(y));
    out.push(...inline);
  }
  return out;
}

/**
 * Where `current` sits in `stops` as a pair of scan starts: `next` is the
 * first index `n` looks at, `prev` the first `p` looks at.
 */
function scanStarts(
  stops: readonly FocusItem[],
  current: FocusItem | null,
): { next: number; prev: number } {
  if (!current) return { next: 0, prev: stops.length - 1 };
  const at = stops.findIndex((s) => sameItem(s, current));
  if (at >= 0) return { next: at + 1, prev: at - 1 };
  // Not a stop (a diff line, a finding just hidden): its card and line.
  const card = cardOf(current);
  const head = stops.findIndex((s) => cardOf(s) === card);
  if (head < 0) return { next: 0, prev: stops.length - 1 };
  const line = lineOf(current);
  let before = head;
  for (let i = head + 1; i < stops.length && cardOf(stops[i]) === card; i++) {
    if (lineOf(stops[i]) < line) before = i;
  }
  return { next: before + 1, prev: before };
}

/**
 * The next (+1) or previous (-1) thread or finding from `current`, or null
 * at the end of the page (no wrap-around).
 */
export function moveFocus(
  stops: readonly FocusItem[],
  current: FocusItem | null,
  delta: 1 | -1,
): FocusItem | null {
  const start = scanStarts(stops, current);
  if (delta > 0) {
    for (let i = start.next; i < stops.length; i++) {
      if (isEntry(stops[i])) return stops[i];
    }
  } else {
    for (let i = start.prev; i >= 0; i--) {
      if (isEntry(stops[i])) return stops[i];
    }
  }
  return null;
}

/** What a screen reader hears for a thread besides its name. */
export function threadDescription(t: StoredThread): string {
  const comments = t.notes.filter((n) => !n.system).length;
  const parts = [`${comments} ${comments === 1 ? "comment" : "comments"}`];
  if (t.mine) parts.push("yours");
  if (t.resolved) parts.push("resolved");
  if (t.outdated) parts.push("outdated");
  const a = t.anchor;
  const path = a?.pathNew ?? a?.pathOld;
  if (path) {
    const line = a?.currentLine ?? a?.lineNew ?? a?.lineOld;
    parts.push(line ? `on ${path} line ${line}` : `on ${path}`);
  } else {
    parts.push("on the review as a whole");
  }
  return parts.join(", ");
}
