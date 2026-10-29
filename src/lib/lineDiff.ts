/**
 * The diff the merge views draw for a large file: lines first, then
 * characters inside the lines that changed — as `git diff` does.
 *
 * Why: `@codemirror/merge` diffs characters with a scan limit (500, its
 * default for the merge views). Past it the library gives up on precision:
 * once the part of a file between its first and last change is longer than
 * 4,000 characters it falls back to a crude match, and past 16,000 the
 * whole stretch becomes one change. A 600-line file with a few dozen hunks
 * then shows every line between the first and the last hunk as changed:
 * nothing collapses, the editor is tens of thousands of pixels tall, and
 * the unified view draws all of its removed lines as one block of DOM.
 * Comparing whole lines first (patience diff: lines unique on both sides
 * anchor the rest, Myers between them, as `git diff --patience`) keeps the
 * hunks the platform shows, and the character diff inside each changed
 * run stays small.
 *
 * Small texts keep the library's own diff, exactly as before. The find bar
 * diffs with the same config (`whitespaceDiff.editorDiffConfig`), so its
 * matches sit on the lines the editors show.
 */

import {
  Change,
  diff,
  presentableDiff,
  type DiffConfig,
} from "@codemirror/merge";

/** The library's own default for the merge views. */
const SCAN: DiffConfig = { scanLimit: 500 };
/**
 * The line search: no scan limit (a few thousand lines are cheap) but a
 * time bound, after which the library's crude match is still a line match.
 */
const LINES: DiffConfig = { timeout: 500 };

/**
 * From this many characters on either side the library's character diff
 * may go crude (`scanLimit / 2 * 16`).
 */
export const LINE_DIFF_FROM = 4000;
/** A changed run longer than this (either side) is one change. */
const RUN_CHARS = 20_000;

/** Each line's start offset. */
function starts(lines: readonly string[]): number[] {
  const out: number[] = [];
  let at = 0;
  for (const l of lines) {
    out.push(at);
    at += l.length + 1;
  }
  return out;
}

/**
 * Lines `[from, to)` as a text range; a run reaching the end takes the
 * line break before it (the last line has none).
 */
function range(
  text: string,
  lineStarts: readonly number[],
  from: number,
  to: number,
): [number, number] {
  const n = lineStarts.length;
  if (from === to) {
    const at = from < n ? lineStarts[from] : text.length;
    return [at, at];
  }
  if (to < n) return [lineStarts[from], lineStarts[to]];
  return [from > 0 ? lineStarts[from] - 1 : 0, text.length];
}

/** One stretch of lines still to compare: `[fa, ta)` of a, `[fb, tb)` of b. */
type Span = [fa: number, ta: number, fb: number, tb: number];

/**
 * Lines that occur exactly once on each side of a span, matched up, longest
 * increasing run of them (patience diff's anchors): `[ia, ib]` pairs,
 * increasing on both sides.
 */
function anchors(
  a: number[],
  b: number[],
  [fa, ta, fb, tb]: Span,
): [number, number][] {
  const seen = new Map<
    number,
    { na: number; nb: number; ia: number; ib: number }
  >();
  for (let i = fa; i < ta; i++) {
    const e = seen.get(a[i]);
    if (e) e.na++;
    else seen.set(a[i], { na: 1, nb: 0, ia: i, ib: -1 });
  }
  for (let i = fb; i < tb; i++) {
    const e = seen.get(b[i]);
    if (!e) continue;
    e.nb++;
    e.ib = i;
  }
  const pairs: [number, number][] = [];
  for (const e of seen.values()) {
    if (e.na === 1 && e.nb === 1) pairs.push([e.ia, e.ib]);
  }
  pairs.sort((x, y) => x[0] - y[0]);
  // Longest increasing subsequence on the b side (patience sorting).
  const tails: number[] = [];
  const prev = new Array<number>(pairs.length).fill(-1);
  for (let i = 0; i < pairs.length; i++) {
    let lo = 0;
    let hi = tails.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (pairs[tails[mid]][1] < pairs[i][1]) lo = mid + 1;
      else hi = mid;
    }
    if (lo > 0) prev[i] = tails[lo - 1];
    tails[lo] = i;
  }
  const out: [number, number][] = [];
  for (
    let i = tails.length ? tails[tails.length - 1] : -1;
    i >= 0;
    i = prev[i]
  ) {
    out.push(pairs[i]);
  }
  return out.reverse();
}

/** The library's (Myers) diff over a span, one character per line. */
function myers(
  a: number[],
  b: number[],
  [fa, ta, fb, tb]: Span,
  out: Change[],
) {
  const codes = new Map<number, string>();
  let next = 0x100;
  const spell = (xs: number[], from: number, to: number): string | null => {
    let s = "";
    for (let i = from; i < to; i++) {
      let c = codes.get(xs[i]);
      if (c === undefined) {
        if (next === 0xd800) next = 0xe000; // no surrogate halves
        if (next > 0xffff) return null;
        c = String.fromCharCode(next++);
        codes.set(xs[i], c);
      }
      s += c;
    }
    return s;
  };
  const sa = spell(a, fa, ta);
  const sb = spell(b, fb, tb);
  if (sa === null || sb === null) {
    out.push(new Change(fa, ta, fb, tb));
    return;
  }
  for (const c of diff(sa, sb, LINES)) {
    out.push(new Change(fa + c.fromA, fa + c.toA, fb + c.fromB, fb + c.toB));
  }
}

/**
 * The changed line runs (in line numbers from 0): patience diff — the lines
 * unique on both sides anchor the rest, and the stretches between anchors
 * are compared the same way, down to the library's diff where no anchor is
 * left. A 20,000-line file with thousands of changed lines diffs in
 * milliseconds; a plain Myers search over it would not.
 */
function lineRuns(a: string[], b: string[]): Change[] {
  const ids = new Map<string, number>();
  const code = (line: string) => {
    let c = ids.get(line);
    if (c === undefined) {
      c = ids.size;
      ids.set(line, c);
    }
    return c;
  };
  const ca = a.map(code);
  const cb = b.map(code);
  const out: Change[] = [];
  const todo: Span[] = [[0, ca.length, 0, cb.length]];
  while (todo.length > 0) {
    let [fa, ta, fb, tb] = todo.pop()!;
    while (fa < ta && fb < tb && ca[fa] === cb[fb]) {
      fa++;
      fb++;
    }
    while (fa < ta && fb < tb && ca[ta - 1] === cb[tb - 1]) {
      ta--;
      tb--;
    }
    if (fa === ta && fb === tb) continue;
    if (fa === ta || fb === tb) {
      out.push(new Change(fa, ta, fb, tb));
      continue;
    }
    const span: Span = [fa, ta, fb, tb];
    const found = anchors(ca, cb, span);
    if (found.length === 0) {
      myers(ca, cb, span, out);
      continue;
    }
    let pa = fa;
    let pb = fb;
    for (const [ia, ib] of found) {
      todo.push([pa, ia, pb, ib]);
      pa = ia + 1;
      pb = ib + 1;
    }
    todo.push([pa, ta, pb, tb]);
  }
  out.sort((x, y) => x.fromA - y.fromA || x.fromB - y.fromB);
  // Touching runs are one.
  const merged: Change[] = [];
  for (const c of out) {
    const last = merged[merged.length - 1];
    if (last && last.toA === c.fromA && last.toB === c.fromB) {
      merged[merged.length - 1] = new Change(
        last.fromA,
        c.toA,
        last.fromB,
        c.toB,
      );
    } else {
      merged.push(c);
    }
  }
  return merged;
}

/** Lines first, then characters inside each changed run. */
export function lineFirstDiff(a: string, b: string): readonly Change[] {
  const la = a.split("\n");
  const lb = b.split("\n");
  const runs = lineRuns(la, lb);
  const sa = starts(la);
  const sb = starts(lb);
  const out: Change[] = [];
  for (const run of runs) {
    const [fa, ta] = range(a, sa, run.fromA, run.toA);
    const [fb, tb] = range(b, sb, run.fromB, run.toB);
    if (fa === ta || fb === tb || ta - fa > RUN_CHARS || tb - fb > RUN_CHARS) {
      out.push(new Change(fa, ta, fb, tb));
      continue;
    }
    // Presentable per run (words whole, close edits joined): the library
    // tidies the whole list again, and joining thousands of small edits
    // there costs a splice each.
    for (const c of presentableDiff(a.slice(fa, ta), b.slice(fb, tb), SCAN)) {
      out.push(new Change(fa + c.fromA, fa + c.toA, fb + c.fromB, fb + c.toB));
    }
  }
  return out;
}

/** The merge views' diff: the library's for small texts, else lines first. */
export function editorDiff(a: string, b: string): readonly Change[] {
  if (a === b) return [];
  if (Math.max(a.length, b.length) < LINE_DIFF_FROM) return diff(a, b, SCAN);
  return lineFirstDiff(a, b);
}
