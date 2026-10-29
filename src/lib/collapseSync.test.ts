import { Text } from "@codemirror/state";
import { describe, expect, it } from "vitest";

import {
  mapCollapseLine,
  mapCollapsePos,
  mirrorPositions,
} from "./collapseSync";

// Base "a\nb\nc\nd\n" (positions: a=0, b=2, c=4, d=6) against head
// "a\nB\nB2\nc\nd\n": line b changed into two lines.
const chunks = [{ fromA: 2, toA: 4, fromB: 2, toB: 7 }];

describe("collapseSync", () => {
  it("maps an unchanged spot across the chunks before it", () => {
    // Before the change: the same offset.
    expect(mapCollapsePos(0, chunks, true)).toBe(0);
    expect(mapCollapsePos(2, chunks, true)).toBe(2);
    // After it: shifted by what the chunk added, both ways.
    expect(mapCollapsePos(4, chunks, true)).toBe(7);
    expect(mapCollapsePos(6, chunks, true)).toBe(9);
    expect(mapCollapsePos(9, chunks, false)).toBe(6);
  });

  it("mirrors each opened stretch once", () => {
    expect(mirrorPositions([6, 6, 0], chunks, true)).toEqual([9, 0]);
    expect(mirrorPositions([], chunks, false)).toEqual([]);
  });

  it("maps by lines when an unchanged line differs in whitespace", () => {
    // Whitespace hidden: line 2 changed; line 3 is reindented but unchanged.
    const a = Text.of(["x", "old", "  y", "z", "w"]);
    const b = Text.of(["x", "new", "new2", "y", "z", "w"]);
    const cs = [
      {
        fromA: a.line(2).from,
        toA: a.line(3).from,
        fromB: b.line(2).from,
        toB: b.line(4).from,
      },
    ];
    // Line 4 ("z") of a is line 5 of b, though "  y" is two characters
    // longer than "y" (the offset mapping lands inside line 4).
    expect(mapCollapseLine(a.line(4).from, cs, true, a, b)).toBe(
      b.line(5).from,
    );
    expect(mapCollapsePos(a.line(4).from, cs, true)).not.toBe(b.line(5).from);
    expect(mapCollapseLine(b.line(5).from, cs, false, b, a)).toBe(
      a.line(4).from,
    );
    expect(
      mirrorPositions([a.line(4).from], cs, true, { ours: a, other: b }),
    ).toEqual([b.line(5).from]);
    // Before any chunk: the same line.
    expect(mapCollapseLine(0, cs, true, a, b)).toBe(0);
  });
});
