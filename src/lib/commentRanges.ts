/**
 * Which lines of a file a new comment can go on, and the ranges a gutter
 * click, shift-click or drag selects. Pure (no React, no IPC).
 *
 * `CommentableLines` (the backend's `commentable_lines`) lists per hunk the
 * old-side lines (removed + context) and new-side lines (added + context).
 * With `outside` other than `none` every other line of a side is
 * commentable too: unchanged code outside the hunks (GitLab takes it as a
 * line comment, GitHub as a file comment quoting the lines).
 *
 * A range stays inside one region: one hunk (GitHub's rule, and GitLab's
 * line_range needs both ends in the same diff), or one stretch of unchanged
 * code between two hunks. The stretches are counted as the core counts
 * them (`post::anchor::region_of`): a line outside the hunks belongs to the
 * gap after every hunk whose lines on that side all lie above it. A
 * selection is clamped to the region of the line it started on.
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

/** Whether lines outside the hunks take comments. */
export function takesOutside(c: CommentableLines | null | undefined): boolean {
  return c?.outside === "native" || c?.outside === "fallback";
}

/**
 * Every commentable line of `side`: the hunks' lines, plus with
 * `takesOutside` every line up to `docLines` (the side's line count; 0
 * leaves the outside lines out).
 */
export function commentableSet(
  c: CommentableLines | null | undefined,
  side: Side,
  docLines = 0,
): Set<number> {
  const out = new Set<number>();
  if (takesOutside(c)) for (let n = 1; n <= docLines; n++) out.add(n);
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

/** The stretch of lines a range may span, and what kind it is. */
export interface LineRegion {
  /** Equal for two lines of the same region (`h<n>` hunk, `g<n>` gap). */
  key: string;
  /** Unchanged code outside the hunks. */
  outside: boolean;
  /** First and last line of the region (`to` may be Infinity). */
  from: number;
  to: number;
}

/**
 * The region of `line` on `side`; null when it takes no comment.
 * `docLines` bounds the stretch after the last hunk (Infinity: unknown).
 */
export function regionOf(
  c: CommentableLines | null | undefined,
  side: Side,
  line: number,
  docLines = Infinity,
): LineRegion | null {
  const hunks = c?.hunks ?? [];
  for (let i = 0; i < hunks.length; i++) {
    const lines = linesOf(hunks[i], side);
    if (lines.includes(line)) {
      return {
        key: `h${i}`,
        outside: false,
        from: Math.min(...lines),
        to: Math.max(...lines),
      };
    }
  }
  if (!takesOutside(c) || line < 1 || line > docLines) return null;
  let above = 0;
  let from = 1;
  let to = docLines;
  for (const h of hunks) {
    const lines = linesOf(h, side);
    if (lines.length === 0) continue;
    const lo = Math.min(...lines);
    const hi = Math.max(...lines);
    if (hi < line) {
      above += 1;
      from = Math.max(from, hi + 1);
    } else if (lo > line) {
      to = Math.min(to, lo - 1);
    }
  }
  return { key: `g${above}`, outside: true, from, to };
}

/** A comment's lines: `startLine` is null for a single line. */
export interface LineRange {
  line: number;
  startLine: number | null;
}

/**
 * The range from `anchor` (where the selection started) to `to`, clamped to
 * the anchor's region; null when the anchor is not commentable.
 */
export function clampRange(
  c: CommentableLines | null | undefined,
  side: Side,
  anchor: number,
  to: number,
  docLines = Infinity,
): LineRange | null {
  const region = regionOf(c, side, anchor, docLines);
  if (!region) return null;
  // Within a region a side's lines are consecutive: the nearest end is
  // the clamp (also for a `to` outside it).
  const target = Math.min(region.to, Math.max(region.from, to));
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

/**
 * What a comment outside the diff becomes, for the composer: null inside
 * the diff.
 */
export function outsideNote(
  outside: CommentableLines["outside"] | null | undefined,
): string | null {
  switch (outside) {
    case "native":
      return "Outside the diff: posted as a comment on these unchanged lines.";
    case "fallback":
      return "Outside the diff: GitHub takes no line comment here, so this posts as a file comment that quotes and links these lines.";
    default:
      return null;
  }
}
