/**
 * Which lines of a file a new comment can go on, and the ranges a gutter
 * click, shift-click or drag selects. Pure (no React, no IPC).
 *
 * `CommentableLines` (the backend's `commentable_lines`) lists per hunk the
 * old-side lines (removed + context) and new-side lines (added + context). A
 * range must stay inside one hunk (GitHub's rule, and GitLab's line_range
 * needs both ends in the same diff): a selection is clamped to the hunk of
 * the line it started on.
 */

import type { CommentableHunk, CommentableLines, Side } from "../ipc/types";
import type { DiffSide } from "./diffThreads";

export function sideOf(s: DiffSide): Side {
  return s === "a" ? "old" : "new";
}

export function diffSideOf(s: Side): DiffSide {
  return s === "old" ? "a" : "b";
}

function linesOf(h: CommentableHunk, side: Side): number[] {
  return side === "old" ? h.old : h.new;
}

/** Every commentable line of `side`. */
export function commentableSet(
  c: CommentableLines | null | undefined,
  side: Side,
): Set<number> {
  const out = new Set<number>();
  for (const h of c?.hunks ?? []) for (const n of linesOf(h, side)) out.add(n);
  return out;
}

/** The lines of the hunk holding `line` on `side`, or null. */
export function hunkLines(
  c: CommentableLines | null | undefined,
  side: Side,
  line: number,
): number[] | null {
  for (const h of c?.hunks ?? []) {
    const lines = linesOf(h, side);
    if (lines.includes(line)) return lines;
  }
  return null;
}

/** A comment's lines: `startLine` is null for a single line. */
export interface LineRange {
  line: number;
  startLine: number | null;
}

/**
 * The range from `anchor` (where the selection started) to `to`, clamped to
 * the anchor's hunk; null when the anchor is not commentable.
 */
export function clampRange(
  c: CommentableLines | null | undefined,
  side: Side,
  anchor: number,
  to: number,
): LineRange | null {
  const hunk = hunkLines(c, side, anchor);
  if (!hunk || hunk.length === 0) return null;
  // Within a hunk a side's lines are consecutive; the nearest one is the
  // clamp (also for a `to` outside the hunk).
  let target = hunk[0];
  for (const n of hunk) {
    if (Math.abs(n - to) < Math.abs(target - to)) target = n;
  }
  const from = Math.min(anchor, target);
  const end = Math.max(anchor, target);
  return { line: end, startLine: from === end ? null : from };
}

/** `L12` or `L10–12`, with the side for old lines. */
export function rangeLabel(side: Side, r: LineRange): string {
  const lines =
    r.startLine !== null && r.startLine !== r.line
      ? `lines ${r.startLine}–${r.line}`
      : `line ${r.line}`;
  return side === "old" ? `${lines} (old)` : lines;
}
