import { Chunk } from "@codemirror/merge";
import { Text } from "@codemirror/state";
import { describe, expect, it } from "vitest";

import { whitespaceKey } from "./whitespace";
import { HIDE_WHITESPACE, ignoreWhitespaceDiff } from "./whitespaceDiff";

const doc = (lines: string[]) => Text.of(lines);

/** The chunks the merge view builds, as 1-based line spans per side. */
function chunks(a: string[], b: string[], hide = true) {
  const ta = doc(a);
  const tb = doc(b);
  return Chunk.build(ta, tb, hide ? HIDE_WHITESPACE : undefined).map((c) => ({
    a: span(ta, c.fromA, c.toA),
    b: span(tb, c.fromB, c.toB),
    /** The highlighted text per change, as the view marks it. */
    changes: c.changes.map((x) => ({
      a: ta.sliceString(c.fromA + x.fromA, c.fromA + x.toA),
      b: tb.sliceString(c.fromB + x.fromB, c.fromB + x.toB),
    })),
  }));
}

/** Lines `[first, last]` a chunk side covers; null when empty. */
function span(t: Text, from: number, to: number): [number, number] | null {
  if (from === to) return null;
  return [t.lineAt(from).number, t.lineAt(Math.min(to - 1, t.length)).number];
}

/**
 * Walk both documents outside the chunks: each unchanged line must pair
 * with one of the same key, and the stretches must be as long on both
 * sides (else the views would drift apart).
 */
function assertAligned(a: string[], b: string[]) {
  const ta = doc(a);
  const tb = doc(b);
  const cs = Chunk.build(ta, tb, HIDE_WHITESPACE);
  let la = 1;
  let lb = 1;
  const walkTo = (endA: number, endB: number) => {
    expect(endA - la).toBe(endB - lb);
    for (; la < endA; la++, lb++) {
      expect(whitespaceKey(a[la - 1])).toBe(whitespaceKey(b[lb - 1]));
    }
  };
  for (const c of cs) {
    walkTo(ta.lineAt(c.fromA).number, tb.lineAt(c.fromB).number);
    la = c.toA > ta.length ? ta.lines + 1 : ta.lineAt(c.toA).number;
    lb = c.toB > tb.length ? tb.lines + 1 : tb.lineAt(c.toB).number;
  }
  walkTo(ta.lines + 1, tb.lines + 1);
}

describe("ignoreWhitespaceDiff", () => {
  it("treats a reindented line as unchanged", () => {
    const a = ["fn main() {", "  run();", "}"];
    const b = ["fn main() {", "    run();", "}"];
    expect(chunks(a, b, false)).toHaveLength(1);
    expect(chunks(a, b)).toEqual([]);
  });

  it("ignores trailing spaces", () => {
    expect(chunks(["a", "b", "c"], ["a  ", "b\t", "c "])).toEqual([]);
  });

  it("ignores CRLF against LF", () => {
    expect(ignoreWhitespaceDiff("a\r\nb\r\nc", "a\nb\nc")).toEqual([]);
  });

  it("ignores tabs against spaces and spaces inside a line", () => {
    expect(
      chunks(
        ["\tif (x) {", "\t\ty = 1;", "\t}"],
        ["  if (x){", "    y=1;", "  }"],
      ),
    ).toEqual([]);
  });

  it("keeps a real edit and skips the whitespace beside it in one hunk", () => {
    const a = ["a", "  b(1)", "c", "d"];
    const b = ["a", "b( 2 )", "  c", "d"];
    const out = chunks(a, b);
    // Only line 2 changed; line 3 (indent only) is not in the chunk.
    expect(out).toHaveLength(1);
    expect(out[0].a).toEqual([2, 2]);
    expect(out[0].b).toEqual([2, 2]);
    // The highlight is the digit, not the spaces around it.
    expect(out[0].changes).toEqual([{ a: "1", b: "2" }]);
    for (const c of out[0].changes) {
      expect(/^\s*$/.test(c.a) && /^\s*$/.test(c.b)).toBe(false);
    }
  });

  it("still shows added and removed blank lines (as git diff -w does)", () => {
    const out = chunks(["a", "b"], ["a", "", "b"]);
    expect(out).toHaveLength(1);
    expect(out[0].a).toBeNull();
    expect(out[0].b).toEqual([2, 2]);
  });

  it("finds nothing in a file whose changes are all whitespace", () => {
    const a = ["class A {", "\tfoo() {", "\t\treturn 1;", "\t}", "}"];
    const b = ["class A {", "    foo() {  ", "        return 1;", "    }", "}"];
    expect(ignoreWhitespaceDiff(a.join("\n"), b.join("\n"))).toEqual([]);
    expect(chunks(a, b)).toEqual([]);
  });

  it("keeps added lines on their lines next to reindented ones", () => {
    // Code wrapped in a block: the body is reindented, the braces are new.
    // The inserted "  }" equals the line after it, which tempts the
    // library's alignment to slide it over the reindented text.
    const a = ["start();", "foo();", "}", "end();"];
    const b = ["start();", "if (x) {", "  foo();", "  }", "  }", "end();"];
    assertAligned(a, b);
    const out = chunks(a, b);
    // Nothing but the new lines: "if (x) {" and one "  }".
    const added = out.flatMap((c) => (c.b ? b.slice(c.b[0] - 1, c.b[1]) : []));
    expect(added.map(whitespaceKey).sort()).toEqual(["if(x){", "}"].sort());
    expect(out.every((c) => c.a === null)).toBe(true);
  });

  it("handles an added file, a removed file and an insertion at the end", () => {
    expect(chunks([""], ["x", "y"])).toHaveLength(1);
    expect(chunks(["x", "y"], [""])).toHaveLength(1);
    assertAligned(["x", "  y"], ["x", "y", "z"]);
    assertAligned(["  x", "y", "z"], ["x", "y"]);
  });

  it("stays aligned on generated edits", () => {
    // A small deterministic generator: lines from a few words, edited by
    // reindenting, retyping, inserting and deleting.
    let seed = 7;
    const rand = (n: number) => {
      seed = (seed * 1103515245 + 12345) % 2 ** 31;
      return seed % n;
    };
    const words = [
      "}",
      "{",
      "foo();",
      "bar(x);",
      "",
      "return y;",
      "  }",
      "a = b + c;",
      "a=b+ d;",
    ];
    const pad = () => [" ", "  ", "\t", ""][rand(4)];
    for (let round = 0; round < 2000; round++) {
      const a = Array.from(
        { length: 3 + rand(12) },
        () => pad() + words[rand(words.length)],
      );
      const b: string[] = [];
      for (const line of a) {
        const op = rand(6);
        if (op === 0) continue; // deleted
        if (op === 1) b.push(words[rand(words.length)]); // retyped
        if (op === 2) b.push(pad() + line.trim() + pad()); // reindented
        if (op === 3) b.push(pad() + words[rand(words.length)]); // inserted
        if (op !== 1) b.push(line);
      }
      if (b.length === 0) b.push("");
      assertAligned(a, b);
      // No chunk is whitespace alone.
      const ta = doc(a);
      const tb = doc(b);
      for (const c of Chunk.build(ta, tb, HIDE_WHITESPACE)) {
        const keys = (t: Text, from: number, to: number) => {
          const s = span(t, from, to);
          if (!s) return [];
          const out: string[] = [];
          for (let n = s[0]; n <= s[1]; n++) {
            out.push(whitespaceKey(t.line(n).text));
          }
          return out;
        };
        expect(keys(ta, c.fromA, c.toA)).not.toEqual(keys(tb, c.fromB, c.toB));
      }
    }
  });
});
