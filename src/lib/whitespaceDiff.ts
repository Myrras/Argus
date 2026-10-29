/**
 * The diff `@codemirror/merge` draws when whitespace changes are hidden: a
 * `DiffConfig.override` (`HIDE_WHITESPACE`), so the merge view keeps both
 * documents exactly as they are (line numbers, threads, comments and
 * suggestions stay on the real lines) and only its chunks change.
 *
 * - Lines are compared with all whitespace ignored (`git diff -w`, GitHub's
 *   `?w=1`; `lib/whitespace.whitespaceKey`): a line that differs only in
 *   indentation, trailing spaces, tabs vs spaces or CRLF is unchanged, so it
 *   is in no chunk and folds with the unchanged code.
 * - Inside the lines that did change, the character diff is the library's
 *   own (`presentableDiff`), minus the spans that are whitespace on both
 *   sides, so the intra-line highlights skip them.
 *
 * Why the extra pass (`settle`): `Chunk.build` runs the library's
 * `makePresentable` over whatever the override returns, and it may slide a
 * pure insertion or deletion over the text after it, assuming that text is
 * the same on both sides. Next to a line that differs only in whitespace it
 * is not, and the slide would land the chunk on the wrong lines. So the
 * result is run through `presentableDiff` once more here; a chunk that
 * comes out different is widened to whole lines (which it leaves alone).
 */

import {
  Change,
  diff,
  presentableDiff,
  type DiffConfig,
} from "@codemirror/merge";
import { Text } from "@codemirror/state";

import { editorDiff } from "./lineDiff";
import { whitespaceKey } from "./whitespace";

/** The library's own default for the merge views. */
const SCAN: DiffConfig = { scanLimit: 500 };
/**
 * The line pass: one character per line, so no scan limit is needed (a
 * few thousand lines are cheap), only a time bound (see `lib/lineDiff`).
 */
const LINES: DiffConfig = { timeout: 500 };

/**
 * The merge views' diff with whitespace shown: the library's own for small
 * texts, lines first for large ones (`lib/lineDiff`).
 */
export const PLAIN_DIFF: DiffConfig = {
  override: (a, b) => editorDiff(a, b),
};

/** The merge view's diff config with whitespace changes hidden. */
export const HIDE_WHITESPACE: DiffConfig = {
  override: (a, b) => ignoreWhitespaceDiff(a, b),
};

interface Lines {
  text: string;
  /** Where each line starts. */
  starts: number[];
  lines: string[];
}

function linesOf(text: string): Lines {
  const lines = text.split("\n");
  const starts: number[] = [];
  let at = 0;
  for (const l of lines) {
    starts.push(at);
    at += l.length + 1;
  }
  return { text, starts, lines };
}

/**
 * Lines `[from, to)` as a text range. A run that reaches the end takes the
 * line break before it instead of after (the last line has none); the
 * other side of the same change then reaches the end too (what follows a
 * change is unchanged on both sides).
 */
function rangeOf(l: Lines, from: number, to: number): [number, number] {
  const n = l.lines.length;
  if (from === to) {
    const at = from < n ? l.starts[from] : l.text.length;
    return [at, at];
  }
  if (to < n) return [l.starts[from], l.starts[to]];
  return [from > 0 ? l.starts[from] - 1 : 0, l.text.length];
}

interface LineChange {
  fromA: number;
  toA: number;
  fromB: number;
  toB: number;
}

/**
 * The changed line runs, comparing lines by key: each distinct key becomes
 * one character and the library's (Myers) diff runs over those. Null when
 * there are more distinct lines than characters to spell them.
 */
function lineChanges(a: Lines, b: Lines): LineChange[] | null {
  const codes = new Map<string, string>();
  let next = 0x100;
  const spell = (lines: string[]): string | null => {
    let out = "";
    for (const line of lines) {
      const key = whitespaceKey(line);
      let c = codes.get(key);
      if (c === undefined) {
        if (next === 0xd800) next = 0xe000; // no surrogate halves
        if (next > 0xffff) return null;
        c = String.fromCharCode(next++);
        codes.set(key, c);
      }
      out += c;
    }
    return out;
  };
  const sa = spell(a.lines);
  const sb = spell(b.lines);
  if (sa === null || sb === null) return null;
  return [...diff(sa, sb, LINES)];
}

/** Only whitespace, with as many line breaks on both sides. */
function blankAlike(x: string, y: string): boolean {
  return (
    /^\s*$/.test(x) &&
    /^\s*$/.test(y) &&
    x.split("\n").length === y.split("\n").length
  );
}

/**
 * `[from, to)` of `text` without the spaces and tabs at its ends (line
 * breaks stay: they are the line structure). The library's diff joins
 * touching edits, so "1" → " 2 " is a digit and two spaces.
 */
function trimmed(text: string, from: number, to: number): [number, number] {
  const blank = (i: number) => /[^\S\n]/.test(text[i]);
  while (from < to && blank(from)) from++;
  while (to > from && blank(to - 1)) to--;
  return [from, to];
}

/** `presentableDiff`'s last step: changes closer than `gap` become one. */
function mergeClose(changes: Change[], gap: number): Change[] {
  const out: Change[] = [];
  for (const c of changes) {
    const prev = out[out.length - 1];
    if (prev && prev.toA > c.fromA - gap && prev.toB > c.fromB - gap) {
      out[out.length - 1] = new Change(prev.fromA, c.toA, prev.fromB, c.toB);
    } else {
      out.push(c);
    }
  }
  return out;
}

/** The changes, before `settle`. */
function rawChanges(a: string, b: string): Change[] | null {
  const la = linesOf(a);
  const lb = linesOf(b);
  const runs = lineChanges(la, lb);
  if (!runs) return null;
  const out: Change[] = [];
  for (const run of runs) {
    const [fa, ta] = rangeOf(la, run.fromA, run.toA);
    const [fb, tb] = rangeOf(lb, run.fromB, run.toB);
    if (fa === ta || fb === tb) {
      out.push(new Change(fa, ta, fb, tb));
      continue;
    }
    // Changed lines: the character diff without its whitespace-only spans
    // (the plain diff: `presentableDiff` would first merge a real edit with
    // the spaces around it; `Chunk.build` tidies what is left).
    for (const c of diff(a.slice(fa, ta), b.slice(fb, tb), SCAN)) {
      const [fx, tx] = trimmed(a, fa + c.fromA, fa + c.toA);
      const [fy, ty] = trimmed(b, fb + c.fromB, fb + c.toB);
      if (blankAlike(a.slice(fx, tx), b.slice(fy, ty))) continue;
      out.push(new Change(fx, tx, fy, ty));
    }
  }
  return mergeClose(out, 3);
}

/** A chunk's line-aligned extent, as `Chunk.build` makes it. */
export interface ChunkExtent {
  fromA: number;
  toA: number;
  fromB: number;
  toB: number;
}

/** `toChunks` of `@codemirror/merge` 6.12: the chunks `changes` make. */
export function chunkExtents(
  changes: readonly Change[],
  a: Text,
  b: Text,
): ChunkExtent[] {
  const fromLine = (fa: number, fb: number): [number, number] => {
    const la = a.lineAt(fa);
    const lb = b.lineAt(fb);
    return la.to === fa && lb.to === fb && fa < a.length && fb < b.length
      ? [fa + 1, fb + 1]
      : [la.from, lb.from];
  };
  const toLine = (ta: number, tb: number): [number, number] => {
    const la = a.lineAt(ta);
    const lb = b.lineAt(tb);
    return la.from === ta && lb.from === tb ? [ta, tb] : [la.to + 1, lb.to + 1];
  };
  const out: ChunkExtent[] = [];
  for (let i = 0; i < changes.length; i++) {
    const c = changes[i];
    const [fromA, fromB] = fromLine(c.fromA, c.fromB);
    let [toA, toB] = toLine(c.toA, c.toB);
    while (i < changes.length - 1) {
      const n = changes[i + 1];
      const [nextA, nextB] = fromLine(n.fromA, n.fromB);
      if (nextA > toA + 1 && nextB > toB + 1) break;
      [toA, toB] = toLine(n.toA, n.toB);
      i++;
    }
    out.push({
      fromA,
      toA: Math.max(fromA, toA),
      fromB,
      toB: Math.max(fromB, toB),
    });
  }
  return out;
}

const keyOf = (c: ChunkExtent) => `${c.fromA}:${c.toA}:${c.fromB}:${c.toB}`;

/**
 * Replace the changes inside `bad` chunks with one whole-line change each.
 * A chunk empty on one side takes the next line along (the previous one at
 * the end), so neither side is empty: `makePresentable` moves only pure
 * insertions and deletions.
 */
function widen(
  changes: readonly Change[],
  bad: readonly ChunkExtent[],
  a: Text,
  b: Text,
): Change[] {
  const wide = bad.map((d) => {
    let { fromA, toA, fromB, toB } = d;
    if (fromA === toA || fromB === toB) {
      if (toA <= a.length && toB <= b.length) {
        toA = a.lineAt(toA).to + 1;
        toB = b.lineAt(toB).to + 1;
      } else if (fromA > 0 && fromB > 0) {
        fromA = a.lineAt(fromA - 1).from;
        fromB = b.lineAt(fromB - 1).from;
      }
    }
    return new Change(
      fromA,
      Math.min(toA, a.length),
      fromB,
      Math.min(toB, b.length),
    );
  });
  const inside = (c: Change) =>
    bad.some(
      (d) =>
        c.fromA >= d.fromA &&
        c.toA <= d.toA &&
        c.fromB >= d.fromB &&
        c.toB <= d.toB,
    );
  const all = [...changes.filter((c) => !inside(c)), ...wide].sort(
    (x, y) => x.fromA - y.fromA || x.fromB - y.fromB,
  );
  // A widened change may now reach into the next one.
  const out: Change[] = [];
  for (const c of all) {
    const prev = out[out.length - 1];
    if (prev && (c.fromA <= prev.toA || c.fromB <= prev.toB)) {
      out[out.length - 1] = new Change(
        prev.fromA,
        Math.max(prev.toA, c.toA),
        prev.fromB,
        Math.max(prev.toB, c.toB),
      );
    } else {
      out.push(c);
    }
  }
  return out;
}

/**
 * A whole-line insertion or deletion moved one line down (`dir` 1) or up
 * (-1) through lines of the same key: "  }" added before "  }" can as well
 * be the second one. Null when it can't move that way.
 */
function slide(c: Change, a: Text, b: Text, dir: 1 | -1): Change | null {
  const ins = c.fromA === c.toA;
  if (ins === (c.fromB === c.toB)) return null;
  // `run` is the side with lines, `at` the side with a position.
  const [run, at] = ins ? [b, a] : [a, b];
  const [rf, rt, p] = ins
    ? [c.fromB, c.toB, c.fromA]
    : [c.fromA, c.toA, c.fromB];
  // Line-aligned, with a line after it on both sides.
  if (rt >= run.length || p >= at.length) return null;
  if (run.lineAt(rf).from !== rf || run.lineAt(rt).from !== rt) return null;
  if (at.lineAt(p).from !== p) return null;
  const first = run.lineAt(rf).number;
  const after = run.lineAt(rt).number;
  const here = at.lineAt(p).number;
  let next: [number, number, number];
  if (dir === 1) {
    if (after >= run.lines || here >= at.lines) return null;
    if (
      whitespaceKey(run.line(first).text) !==
      whitespaceKey(run.line(after).text)
    ) {
      return null;
    }
    next = [
      run.line(first + 1).from,
      run.line(after + 1).from,
      at.line(here + 1).from,
    ];
  } else {
    if (first <= 1 || here <= 1) return null;
    if (
      whitespaceKey(run.line(first - 1).text) !==
      whitespaceKey(run.line(after - 1).text)
    ) {
      return null;
    }
    next = [
      run.line(first - 1).from,
      run.line(after - 1).from,
      at.line(here - 1).from,
    ];
  }
  const [f, t, q] = next;
  return ins ? new Change(q, q, f, t) : new Change(f, t, q, q);
}

/** Keep the chunks `makePresentable` would otherwise move (see the top). */
function settle(a: string, b: string, changes: Change[]): Change[] {
  const ta = Text.of(a.split("\n"));
  const tb = Text.of(b.split("\n"));
  let cur = changes;
  // Chunks already moved once: widened if they still come out wrong.
  const moved = new Set<string>();
  for (let round = 0; round < 4; round++) {
    const after = presentableDiff(a, b, { override: () => cur.slice() });
    const got = new Set(chunkExtents(after, ta, tb).map(keyOf));
    const bad = chunkExtents(cur, ta, tb).filter((c) => !got.has(keyOf(c)));
    if (bad.length === 0) return cur;
    const wide: ChunkExtent[] = [];
    for (const d of bad) {
      const own = cur.filter(
        (c) =>
          c.fromA >= d.fromA &&
          c.toA <= d.toA &&
          c.fromB >= d.fromB &&
          c.toB <= d.toB,
      );
      const alt =
        own.length === 1 && !moved.has(keyOf(d))
          ? (slide(own[0], ta, tb, 1) ?? slide(own[0], ta, tb, -1))
          : null;
      if (!alt) {
        wide.push(d);
        continue;
      }
      cur = cur.map((c) => (c === own[0] ? alt : c));
      for (const e of chunkExtents([alt], ta, tb)) moved.add(keyOf(e));
    }
    if (wide.length) cur = widen(cur, wide, ta, tb);
  }
  // Last resort: every chunk whole lines, which nothing moves.
  return widen(cur, chunkExtents(cur, ta, tb), ta, tb);
}

/**
 * The diff config the editors use, for `Chunk.build` outside them (the find
 * search): its chunks must be the ones on screen.
 */
export function editorDiffConfig(hideWhitespace: boolean): DiffConfig {
  return hideWhitespace ? HIDE_WHITESPACE : PLAIN_DIFF;
}

/**
 * The changes between `a` and `b` with whitespace-only differences left
 * out (both "\n"-separated, as the merge view hands them over).
 */
export function ignoreWhitespaceDiff(a: string, b: string): readonly Change[] {
  if (a === b) return [];
  const raw = rawChanges(a, b);
  // Too many distinct lines to spell: the plain diff.
  if (!raw) return editorDiff(a, b);
  return settle(a, b, raw);
}
