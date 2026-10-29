/**
 * Keeping the two editors of the side-by-side view in step when a collapsed
 * stretch of unchanged lines opens. Pure (no DOM).
 *
 * `@codemirror/merge` collapses unchanged code per editor and opens a
 * stretch with `uncollapseUnchanged.of(pos)`, where `pos` is the stretch's
 * start in that editor. Its own "N unchanged lines" click opens the
 * sibling's too, but an expansion Argus dispatches (to show a thread, the
 * composer, the focused line, the line being read) reached one side only,
 * and a click that Argus' own mousedown handling expanded first never got
 * to the library's handler: the two sides drifted apart. `DiffEditor`
 * therefore mirrors every expansion to the sibling, mapping the position
 * across the changed chunks as the library does (`mapPos` in
 * `@codemirror/merge`, 6.12), but by lines when both documents are given
 * (`mapCollapseLine`): with whitespace changes hidden an unchanged line may
 * differ in whitespace, so only the line count carries over.
 */

/** The parts of a `@codemirror/merge` `Chunk` the mapping reads. */
export interface ChunkSpan {
  fromA: number;
  toA: number;
  fromB: number;
  toB: number;
}

/**
 * `pos` in one editor (`fromA`: the base, left) to the same unchanged spot
 * in the other: unchanged text is identical on both sides, so its offset
 * from the end of the last chunk before it carries over.
 */
export function mapCollapsePos(
  pos: number,
  chunks: readonly ChunkSpan[],
  fromA: boolean,
): number {
  let startOur = 0;
  let startOther = 0;
  for (const ch of chunks) {
    if ((fromA ? ch.fromA : ch.fromB) >= pos) break;
    startOur = fromA ? ch.toA : ch.toB;
    startOther = fromA ? ch.toB : ch.toA;
  }
  return startOther + (pos - startOur);
}

/** The parts of a CodeMirror `Text` the line mapping reads. */
export interface LineDoc {
  readonly length: number;
  readonly lines: number;
  lineAt(pos: number): { number: number };
  line(n: number): { from: number };
}

/**
 * `pos` (a line start in `ours`) to the start of the same unchanged line in
 * `other`: the line's distance from the end of the last chunk before it
 * carries over.
 */
export function mapCollapseLine(
  pos: number,
  chunks: readonly ChunkSpan[],
  fromA: boolean,
  ours: LineDoc,
  other: LineDoc,
): number {
  const lineOf = (doc: LineDoc, at: number) =>
    at > doc.length ? doc.lines + 1 : doc.lineAt(at).number;
  let startOur = 1;
  let startOther = 1;
  for (const ch of chunks) {
    if ((fromA ? ch.fromA : ch.fromB) >= pos) break;
    startOur = lineOf(ours, fromA ? ch.toA : ch.toB);
    startOther = lineOf(other, fromA ? ch.toB : ch.toA);
  }
  const n = startOther + (lineOf(ours, pos) - startOur);
  return other.line(Math.min(Math.max(n, 1), other.lines)).from;
}

/**
 * The sibling's positions for the stretches opened at `positions`; by
 * lines when the two documents are given.
 */
export function mirrorPositions(
  positions: readonly number[],
  chunks: readonly ChunkSpan[],
  fromA: boolean,
  docs?: { ours: LineDoc; other: LineDoc },
): number[] {
  return [
    ...new Set(
      positions.map((p) =>
        docs
          ? mapCollapseLine(p, chunks, fromA, docs.ours, docs.other)
          : mapCollapsePos(p, chunks, fromA),
      ),
    ),
  ];
}
