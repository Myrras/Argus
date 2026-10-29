/**
 * "Hide whitespace changes" without CodeMirror: line keys, whether two
 * texts differ in whitespace only, and a file's +/− counts with whitespace
 * hidden, read from the platform's patch. Pure; the tree and the file cards
 * use it before (or without) loading a file's contents, so it stays out of
 * the lazily loaded CodeMirror bundle (`lib/whitespaceDiff` is the diff).
 *
 * Semantics are `git diff -w` (GitHub's `?w=1`): all whitespace is ignored
 * when comparing lines (indentation, trailing spaces, tabs vs spaces, a
 * space added inside a line, CRLF vs LF), so such lines count as unchanged.
 * A blank line added or removed still counts (that would be
 * `--ignore-blank-lines`, which neither platform's toggle does).
 */

import type { FileEntry } from "../ipc/types";

/** A line's text for comparing with whitespace ignored. */
export function whitespaceKey(line: string): string {
  return line.replace(/\s+/g, "");
}

/** The lines' keys, split on "\n" (a "\r" left over is whitespace). */
function keysOf(text: string): string[] {
  return text.split("\n").map(whitespaceKey);
}

/**
 * The two texts differ, but only in whitespace: every line pairs with one
 * that is the same with whitespace ignored.
 */
export function onlyWhitespaceDiffers(a: string, b: string): boolean {
  if (a === b) return false;
  const ka = keysOf(a);
  const kb = keysOf(b);
  return ka.length === kb.length && ka.every((k, i) => k === kb[i]);
}

/**
 * How many of `a`'s and `b`'s lines are left unmatched by a longest common
 * subsequence of their keys: the removed and added lines with whitespace
 * hidden. Null when the table would be too large to build quickly.
 */
export function unmatchedLines(
  a: readonly string[],
  b: readonly string[],
): { removed: number; added: number } | null {
  // Common ends first: most hunks are a few changed lines in context.
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) {
    start++;
  }
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const n = endA - start;
  const m = endB - start;
  if (n === 0 || m === 0) return { removed: n, added: m };
  if (n * m > MAX_CELLS) return null;
  // One row at a time: the length of the longest common subsequence.
  let prev = new Uint32Array(m + 1);
  let cur = new Uint32Array(m + 1);
  for (let i = 1; i <= n; i++) {
    for (let j = 1; j <= m; j++) {
      cur[j] =
        a[start + i - 1] === b[start + j - 1]
          ? prev[j - 1] + 1
          : Math.max(prev[j], cur[j - 1]);
    }
    [prev, cur] = [cur, prev];
  }
  const common = prev[m];
  return { removed: n - common, added: m - common };
}

const MAX_CELLS = 4_000_000;

/** A file's changes as seen with whitespace hidden. */
export interface WhitespaceStats {
  additions: number;
  deletions: number;
  /** It changed, and every change is whitespace. */
  whitespaceOnly: boolean;
}

/**
 * The +/− counts of a unified patch with whitespace hidden, hunk by hunk:
 * each hunk's old lines (context and removed) against its new lines
 * (context and added), compared by key. Null for a patch it cannot read.
 */
export function patchWhitespaceStats(patch: string): {
  additions: number;
  deletions: number;
} | null {
  let additions = 0;
  let deletions = 0;
  let old: string[] | null = null;
  let neu: string[] = [];
  const flush = (): boolean => {
    if (!old) return true;
    const u = unmatchedLines(old, neu);
    if (!u) return false;
    additions += u.added;
    deletions += u.removed;
    return true;
  };
  for (const line of patch.split("\n")) {
    if (line.startsWith("@@")) {
      if (!flush()) return null;
      old = [];
      neu = [];
      continue;
    }
    if (!old) continue;
    // "\ No newline at end of file" and the like.
    if (line.startsWith("\\")) continue;
    const text = whitespaceKey(line.slice(1));
    if (line.startsWith("+")) neu.push(text);
    else if (line.startsWith("-")) old.push(text);
    else {
      old.push(text);
      neu.push(text);
    }
  }
  return flush() ? { additions, deletions } : null;
}

const cache = new WeakMap<FileEntry, WhitespaceStats | null>();

/**
 * A listed file's changes with whitespace hidden, from its patch; null
 * without one (binary, too large, not sent): the platform's counts stand.
 */
export function fileWhitespaceStats(f: FileEntry): WhitespaceStats | null {
  if (cache.has(f)) return cache.get(f)!;
  let out: WhitespaceStats | null = null;
  if (f.patch && !f.binary) {
    const s = patchWhitespaceStats(f.patch);
    if (s) {
      out = {
        ...s,
        // An added, removed or plainly renamed file is no whitespace edit.
        whitespaceOnly:
          (f.status === "modified" || f.status === "renamed") &&
          f.additions + f.deletions > 0 &&
          s.additions + s.deletions === 0,
      };
    }
  }
  cache.set(f, out);
  return out;
}

/** The +/− a file shows: with whitespace hidden, from its patch if it has one. */
export function shownLineStats(
  f: FileEntry,
  hideWhitespace: boolean,
): { additions: number; deletions: number } {
  const s = (hideWhitespace ? fileWhitespaceStats(f) : null) ?? f;
  return { additions: s.additions, deletions: s.deletions };
}
