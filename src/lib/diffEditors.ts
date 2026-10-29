/**
 * The mounted diffs, by file path, so a keyboard move can ask the right one
 * to bring a block (a thread, a finding) into view. CodeMirror renders only
 * the lines in or near the viewport, so a block far down a long file has no
 * DOM until its editor scrolls there (`lib/revealItem`). No CodeMirror
 * import here: the editor itself loads lazily.
 */

/** What a mounted `DiffEditor` offers the reveal. */
export interface DiffRevealer {
  /**
   * Scroll the editor (and the page column around it) to the line holding
   * block `key`; false when this editor does not hold that block (yet).
   */
  reveal: (key: string) => boolean;
  /**
   * Scroll to line `line` of `side` (the find bar's match), opening the
   * collapsed stretch around it (both sides of a split view); in the
   * unified view a removed line is the block above its chunk. False when
   * the editor is not up yet.
   */
  revealLine?: (side: "old" | "new", line: number) => boolean;
  /** The head line at the page column's reading edge, if inside this diff. */
  readingLine?: () => number | null;
}

/** A thread's block key in a `DiffEditor`. */
export function threadBlockKey(threadId: string): string {
  return `t:${threadId}`;
}

/** A finding's block key (`FileDiffCard`'s line item). */
export function findingBlockKey(findingId: string): string {
  return `finding:${findingId}`;
}

const editors = new Map<string, DiffRevealer>();

/** Register the diff of `path`; returns the unregister. */
export function registerDiffEditor(
  path: string,
  editor: DiffRevealer,
): () => void {
  editors.set(path, editor);
  return () => {
    // A remount (another layout) may have registered its own already.
    if (editors.get(path) === editor) editors.delete(path);
  };
}

/** The mounted diff of `path`, if any. */
export function diffEditorFor(path: string): DiffRevealer | null {
  return editors.get(path) ?? null;
}

// -- Block heights ----------------------------------------------------------

/**
 * The last measured height of each line's blocks, by their keys. Every new
 * placement (a finding dismissed, threads re-read, a draft saved) rebuilds
 * the editor's block widgets; CodeMirror keeps measured heights only for
 * the ones it draws, and estimates the others (above the viewport) at one
 * line. That shrinks the page above what the user reads, so it jumps.
 * A rebuilt widget estimates itself at this height instead.
 */
const heights = new Map<string, number>();
/** Enough for every block of a few large reviews; oldest dropped first. */
const MAX_HEIGHTS = 2000;

/** The air kept right of the blocks, inside the editor's visible part. */
const BLOCK_RIGHT_GAP = 8;

/**
 * How wide the blocks under a line are: the editor's visible width less its
 * gutter and a gap. No floor: a side of a split view can be narrower than
 * any minimum, and a block wider than its column is clipped by it (the
 * block's own content wraps or scrolls inside it).
 */
export function blockWidth(visibleWidth: number, gutterWidth: number): number {
  return Math.max(0, visibleWidth - gutterWidth - BLOCK_RIGHT_GAP);
}

/** One line's blocks as a cache key. */
export function blockSignature(keys: readonly string[]): string {
  return keys.join("\n");
}

export function rememberBlockHeight(signature: string, height: number): void {
  if (!(height > 0)) return;
  heights.delete(signature);
  heights.set(signature, height);
  if (heights.size > MAX_HEIGHTS) {
    const oldest = heights.keys().next().value;
    if (oldest !== undefined) heights.delete(oldest);
  }
}

/** The measured height of these blocks, or -1 (CodeMirror's "unknown"). */
export function blockHeight(signature: string): number {
  return heights.get(signature) ?? -1;
}

// -- Opened stretches ---------------------------------------------------------

/**
 * The folded stretches of unchanged code the user (or a thread, the
 * composer, a reveal) opened, per editor: the review page unmounts diffs
 * far from the viewport (`FileDiffCard`), and a diff mounted again opens
 * them again rather than folding what the reader had opened. Keyed by the
 * editor's file, side, layout, whitespace mode and contents
 * (`stretchKey`): a stretch is a position in one document as one diff
 * folds it.
 */
const opened = new Map<string, Set<number>>();
/** Enough for every diff of a few large reviews; oldest dropped first. */
const MAX_OPENED = 2000;

/** A short fingerprint of a document: its length and its two ends. */
function fingerprint(text: string): string {
  return `${text.length}:${text.slice(0, 24)}:${text.slice(-24)}`;
}

export function stretchKey(
  path: string,
  view: number,
  layout: string,
  hideWhitespace: boolean,
  original: string,
  modified: string,
): string {
  return [
    path,
    view,
    layout,
    hideWhitespace ? "w" : "",
    fingerprint(original),
    fingerprint(modified),
  ].join("\u0000");
}

/** Note that the stretches starting at `positions` were opened. */
export function rememberOpened(key: string, positions: readonly number[]) {
  if (positions.length === 0) return;
  const set = opened.get(key) ?? new Set<number>();
  for (const p of positions) set.add(p);
  opened.delete(key);
  opened.set(key, set);
  if (opened.size > MAX_OPENED) {
    const oldest = opened.keys().next().value;
    if (oldest !== undefined) opened.delete(oldest);
  }
}

/** The stretches opened in this editor before, to open again. */
export function openedStretches(key: string): number[] {
  return [...(opened.get(key) ?? [])];
}

/** Tests: forget every opened stretch. */
export function clearOpenedStretches(): void {
  opened.clear();
}
