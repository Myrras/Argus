/**
 * Where a review's threads go in the code view. Pure (no React, no IPC).
 *
 * The pragmatic outdated-thread rule: a thread the platform calls outdated
 * is never placed in the current diff (its line may now hold other code);
 * it is listed in an "Outdated threads" section at the top of its file, with
 * GitHub's `diff_hunk` (or GitLab's line + commit) as the snippet and a link
 * to its then/now history in the Threads tab. A current thread goes under
 * its line; one that cannot be placed (a file-level comment, a line past the
 * end, a base-side line in "since my last review" mode) is listed at the top
 * too, without the badge.
 */

import type { Text } from "@codemirror/state";

import type { FileEntry, StoredThread } from "../ipc/types";

/** One side of the split view: `a` the base (left), `b` the head (right). */
export type DiffSide = "a" | "b";

/** A thread under a line of one side (1-based). */
export interface Placement {
  thread: StoredThread;
  side: DiffSide;
  line: number;
}

export interface FileThreads {
  /** Current threads with a line. */
  inline: Placement[];
  /** Threads the platform calls outdated. */
  outdated: StoredThread[];
  /** Current threads without a usable line. */
  unplaced: StoredThread[];
}

/** The anchored threads of `file` (matched on either path of a rename). */
export function threadsOnFile(
  threads: readonly StoredThread[],
  file: Pick<FileEntry, "path" | "oldPath">,
): StoredThread[] {
  const old = file.oldPath ?? file.path;
  return threads.filter((t) => {
    const a = t.anchor;
    if (!a) return false;
    return a.pathNew === file.path || a.pathOld === old;
  });
}

/** Total and mine, for the file tree and the card header. */
export function threadCounts(threads: readonly StoredThread[]): {
  total: number;
  mine: number;
} {
  return {
    total: threads.length,
    mine: threads.filter((t) => t.mine).length,
  };
}

/**
 * Split a file's threads into placed / outdated / unplaced. `since`: the
 * left side is the last-reviewed commit, not the base the old-side lines
 * refer to, so those are not placed.
 */
export function placeThreads(
  threads: readonly StoredThread[],
  opts: { since: boolean },
): FileThreads {
  const out: FileThreads = { inline: [], outdated: [], unplaced: [] };
  const byTime = [...threads].sort((x, y) =>
    x.createdAt < y.createdAt ? -1 : x.createdAt > y.createdAt ? 1 : 0,
  );
  for (const t of byTime) {
    const a = t.anchor;
    if (!a) continue;
    if (t.outdated) {
      out.outdated.push(t);
      continue;
    }
    const side: DiffSide = a.side === "old" ? "a" : "b";
    const line =
      a.currentLine ?? (side === "a" ? a.lineOld : (a.lineNew ?? a.lineOld));
    if (line === null || line < 1 || (side === "a" && opts.since)) {
      out.unplaced.push(t);
      continue;
    }
    out.inline.push({ thread: t, side, line });
  }
  return out;
}

/**
 * Drop placements past the end of their side (`lines` per side; 0 for a
 * side that does not exist, an added file's base). Returns what fits and
 * the threads that do not.
 */
export function fitPlacements(
  placements: readonly Placement[],
  lines: Record<DiffSide, number>,
): { fit: Placement[]; rest: StoredThread[] } {
  const fit: Placement[] = [];
  const rest: StoredThread[] = [];
  for (const p of placements) {
    if (p.line <= lines[p.side]) fit.push(p);
    else rest.push(p.thread);
  }
  return { fit, rest };
}

/** A changed range between the two documents (the merge package's Chunk). */
export interface ChunkLike {
  fromA: number;
  toA: number;
  fromB: number;
  toB: number;
}

/**
 * The line of `b` a base-side line shows at in the unified view: inside a
 * changed chunk, the chunk's last line on `b` (the deleted lines render as
 * a block in front of the chunk); elsewhere the same unchanged line on `b`.
 */
export function mapOldLine(
  chunks: readonly ChunkLike[],
  a: Text,
  b: Text,
  line: number,
): number {
  const pos = a.line(Math.min(Math.max(line, 1), a.lines)).from;
  let startA = 0;
  let startB = 0;
  for (const c of chunks) {
    if (pos < c.fromA) break;
    // `toA` is the start of the line after the chunk.
    if (pos >= c.fromA && pos < c.toA) {
      const end = Math.max(c.fromB, Math.min(b.length, c.toB - 1));
      return b.lineAt(end).number;
    }
    startA = c.toA;
    startB = c.toB;
  }
  return b.lineAt(Math.min(b.length, startB + (pos - startA))).number;
}

/**
 * The unchanged stretches the merge package collapses (mirrors its
 * `buildCollapsedRanges`), as document positions, so a thread inside one
 * can uncollapse it (`uncollapseUnchanged` takes the range's start).
 */
export function collapsedRanges(
  chunks: readonly ChunkLike[],
  doc: Text,
  side: DiffSide,
  margin: number,
  minSize: number,
): { from: number; to: number }[] {
  const out: { from: number; to: number }[] = [];
  let prevLine = 1;
  for (let i = 0; ; i++) {
    const chunk = i < chunks.length ? chunks[i] : null;
    const from = i ? prevLine + margin : 1;
    const to = chunk
      ? doc.lineAt(side === "a" ? chunk.fromA : chunk.fromB).number - 1 - margin
      : doc.lines;
    if (to - from + 1 >= minSize) {
      out.push({ from: doc.line(from).from, to: doc.line(to).to });
    }
    if (!chunk) break;
    prevLine = doc.lineAt(
      Math.min(doc.length, side === "a" ? chunk.toA : chunk.toB),
    ).number;
  }
  return out;
}
