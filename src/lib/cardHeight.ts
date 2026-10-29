/**
 * How tall a file card's body is while its diff is not mounted (the review
 * page mounts diffs only near the viewport; see `FileDiffCard`): the height
 * it had when last drawn, else an estimate from its patch. A placeholder of
 * that height keeps the scrollbar and every card's position steady as diffs
 * mount and unmount.
 */

import type { FileEntry } from "../ipc/types";

/** One editor line (`lib/codemirror`: 1.55 × the 13px code font). */
export const LINE_PX = 20;
/** A folded stretch of unchanged lines. */
export const FOLD_PX = 26;
/** A thread or other block under a line, closed. */
export const BLOCK_PX = 96;
/** What a card shows besides the editor (padding, borders). */
const CHROME_PX = 8;
/** A body without an editor (binary, large, failed): one short line. */
export const NOTE_PX = 52;

/** Lines `@@ -a,b +c,d @@` hunks span, per side, and their count. */
function hunkRows(
  patch: string,
  split: boolean,
): { rows: number; hunks: number } {
  let rows = 0;
  let hunks = 0;
  let removed = 0;
  let added = 0;
  const flush = () => {
    // Side by side a change takes its longer side; unified both.
    rows += split ? Math.max(removed, added) : removed + added;
    removed = 0;
    added = 0;
  };
  for (const line of patch.split("\n")) {
    if (line.startsWith("@@")) {
      flush();
      hunks++;
    } else if (line.startsWith("-")) {
      removed++;
    } else if (line.startsWith("+")) {
      added++;
    } else if (line.startsWith(" ") || line === "") {
      flush();
      if (line !== "") rows++;
    }
  }
  flush();
  return { rows, hunks };
}

/**
 * The estimated body height of `f`'s card with its diff drawn: the hunks'
 * rows, a fold between and around them, and its threads closed.
 */
export function estimateBodyHeight(
  f: FileEntry,
  opts: { split: boolean; threads: number; large: boolean },
): number {
  if (f.binary || opts.large) return NOTE_PX + opts.threads * BLOCK_PX;
  if (!f.patch) {
    return CHROME_PX + (f.additions + f.deletions) * LINE_PX;
  }
  const { rows, hunks } = hunkRows(f.patch, opts.split);
  const folds =
    f.status === "modified" || f.status === "renamed" ? hunks + 1 : 0;
  return CHROME_PX + rows * LINE_PX + folds * FOLD_PX + opts.threads * BLOCK_PX;
}

/** Last drawn body heights, by `review|path|layout|whitespace`. */
const measured = new Map<string, number>();
/** Enough for a few huge reviews; oldest dropped first. */
const MAX_MEASURED = 5000;

export function heightKey(
  reviewId: string,
  path: string,
  layout: string,
  hideWhitespace: boolean,
): string {
  return `${reviewId}|${path}|${layout}|${hideWhitespace ? "w" : ""}`;
}

export function rememberHeight(key: string, height: number): void {
  if (!(height > 0)) return;
  measured.delete(key);
  measured.set(key, height);
  if (measured.size > MAX_MEASURED) {
    const oldest = measured.keys().next().value;
    if (oldest !== undefined) measured.delete(oldest);
  }
}

export function measuredHeight(key: string): number | null {
  return measured.get(key) ?? null;
}

/** Tests: forget every measured height. */
export function clearMeasuredHeights(): void {
  measured.clear();
}
