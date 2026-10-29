import { Chunk, diff } from "@codemirror/merge";
import { Text } from "@codemirror/state";
import { describe, expect, it } from "vitest";

import { editorDiff, LINE_DIFF_FROM, lineFirstDiff } from "./lineDiff";
import { editorDiffConfig } from "./whitespaceDiff";

/** A 600-line file and the same with a changed line every 30. */
function bigPair(): [string, string] {
  const base: string[] = [];
  const head: string[] = [];
  for (let n = 0; n < 600; n++) {
    const line = `  const value${n} = compute(item${n % 7}, ${n});`;
    base.push(line);
    head.push(n % 30 === 10 ? `  const changed${n} = other(${n});` : line);
  }
  return [base.join("\n") + "\n", head.join("\n") + "\n"];
}

/** Apply `changes` (from `a` to `b`) to `a`. */
function apply(
  a: string,
  b: string,
  changes: readonly {
    fromA: number;
    toA: number;
    fromB: number;
    toB: number;
  }[],
) {
  let out = "";
  let at = 0;
  for (const c of changes) {
    out += a.slice(at, c.fromA) + b.slice(c.fromB, c.toB);
    at = c.toA;
  }
  return out + a.slice(at);
}

function chunks(a: string, b: string, config = editorDiffConfig(false)) {
  return Chunk.build(Text.of(a.split("\n")), Text.of(b.split("\n")), config);
}

describe("editorDiff", () => {
  it("is the library's own diff for small texts", () => {
    const a = "one\ntwo\nthree\n";
    const b = "one\n2\nthree\nfour\n";
    expect(editorDiff(a, b)).toEqual(diff(a, b, { scanLimit: 500 }));
  });

  it("keeps a large file's hunks apart where the library gives up", () => {
    const [a, b] = bigPair();
    expect(a.length).toBeGreaterThan(LINE_DIFF_FROM * 4);
    // The library alone: everything between the first and last change is
    // one chunk.
    expect(chunks(a, b, { scanLimit: 500 }).length).toBe(1);
    const ours = chunks(a, b);
    expect(ours).toHaveLength(20);
    // Each chunk is the one changed line.
    const doc = Text.of(b.split("\n"));
    for (const c of ours) {
      expect(doc.lineAt(c.fromB).number).toBe(doc.lineAt(c.toB - 1).number);
    }
  });

  it("turns the base into the head", () => {
    const [a, b] = bigPair();
    const extra = b.replace("value100 ", "value100b ") + "tail line\n";
    for (const [x, y] of [
      [a, b],
      [a, extra],
      [extra, a],
      [a, ""],
      ["", a],
    ]) {
      expect(apply(x, y, editorDiff(x, y))).toBe(y);
    }
  });

  it("diffs a 20,000-line file with thousands of changes quickly and by line", () => {
    const base: string[] = [];
    const head: string[] = [];
    for (let n = 0; n < 20_000; n++) {
      const line = `  let v${n} = f(${n % 13});`;
      base.push(line);
      head.push(n % 30 === 10 ? `  let w${n} = g(${n});` : line);
      // Repeated lines too, as code has (closing braces).
      if (n % 7 === 0) {
        base.push("  }");
        head.push("  }");
      }
    }
    const a = base.join("\n") + "\n";
    const b = head.join("\n") + "\n";
    const t0 = performance.now();
    const changes = lineFirstDiff(a, b);
    expect(performance.now() - t0).toBeLessThan(2000);
    expect(apply(a, b, changes)).toBe(b);
    expect(chunks(a, b)).toHaveLength(Math.ceil(20_000 / 30));
  });

  it("matches repeated lines without unique anchors", () => {
    const a = Array.from({ length: 400 }, () => "}").join("\n") + "\n";
    const b = a.replace("}\n}\n", "}\nx\n}\n");
    expect(apply(a, b, editorDiff(a, b))).toBe(b);
  });
});
