import { describe, expect, it } from "vitest";

import { CONVERSATION } from "../state/codeViewStore";
import { finding } from "../test/aiFixtures";
import { file, thread } from "../test/fixtures";
import {
  CONVERSATION_CARD,
  moveFocus,
  pageStops,
  threadDescription,
  type FocusItem,
  type PageInput,
} from "./focusModel";

const anchorAt = (path: string, line: number, side: "old" | "new" = "new") => ({
  pathOld: path,
  pathNew: path,
  lineOld: side === "old" ? line : null,
  lineNew: side === "new" ? line : null,
  startLine: null,
  side,
  baseSha: null,
  startSha: null,
  headSha: "c".repeat(40),
  diffHunk: null,
  currentLine: null,
  outsideDiff: false,
});

const general = thread({ author: "carol-fixture", anchor: null });
const mineGeneral = thread({ mine: true, anchor: null });
const onA10 = thread({ anchor: anchorAt("src/a.rs", 10) });
const onA3 = thread({ anchor: anchorAt("src/a.rs", 3) });
const outdatedA = thread({ outdated: true, anchor: anchorAt("src/a.rs", 7) });
const onB5 = thread({ anchor: anchorAt("src/b.rs", 5) });
const elsewhere = thread({ anchor: anchorAt("docs/gone.md", 1) });
const findA10 = finding({ path: "src/a.rs", line: 10 });
const findB1 = finding({ path: "src/b.rs", line: 1 });

const files = [file({ path: "src/a.rs" }), file({ path: "src/b.rs" })];

function input(over: Partial<PageInput> = {}): PageInput {
  return {
    threads: [onA10, general, onB5, outdatedA, onA3, mineGeneral, elsewhere],
    files,
    failed: false,
    order: [CONVERSATION, "src/a.rs", "src/b.rs"],
    since: false,
    findings: [findA10, findB1],
    ...over,
  };
}

/** A stop as a short name for the order assertions. */
function name(s: FocusItem): string {
  switch (s.kind) {
    case "conversation":
      return "conversation";
    case "file":
      return `file ${s.path}`;
    case "thread":
      return `thread ${s.threadId}`;
    case "finding":
      return `finding ${s.findingId}`;
    case "line":
      return `line ${s.path}:${s.line}`;
  }
}

describe("pageStops", () => {
  it("reads the page top to bottom", () => {
    expect(CONVERSATION_CARD).toBe(CONVERSATION);
    expect(pageStops(input()).map(name)).toEqual([
      "conversation",
      // Mine first, then everyone else's; then "On other files".
      `thread ${mineGeneral.threadId}`,
      `thread ${general.threadId}`,
      `thread ${elsewhere.threadId}`,
      "file src/a.rs",
      // Listed at the top (outdated), then by line; a thread before a
      // finding on the same line.
      `thread ${outdatedA.threadId}`,
      `thread ${onA3.threadId}`,
      `thread ${onA10.threadId}`,
      `finding ${findA10.id}`,
      "file src/b.rs",
      `finding ${findB1.id}`,
      `thread ${onB5.threadId}`,
    ]);
  });

  it("orders the Conversation as its groups: unresolved, comments, resolved", () => {
    const done = thread({ anchor: null, resolvable: true, resolved: true });
    const open = thread({ anchor: null, resolvable: true });
    const awayDone = thread({
      resolvable: true,
      resolved: true,
      anchor: anchorAt("docs/gone.md", 2),
    });
    const stops = pageStops(
      input({
        threads: [done, general, open, mineGeneral, awayDone, elsewhere],
        order: [CONVERSATION],
      }),
    );
    expect(stops.map(name)).toEqual([
      "conversation",
      `thread ${open.threadId}`,
      `thread ${mineGeneral.threadId}`,
      `thread ${general.threadId}`,
      `thread ${done.threadId}`,
      // "On other files" groups the same way.
      `thread ${elsewhere.threadId}`,
      `thread ${awayDone.threadId}`,
    ]);
  });

  it("follows the shown order and leaves out hidden files", () => {
    const stops = pageStops(input({ order: [CONVERSATION, "src/b.rs"] }));
    expect(stops.map(name)).not.toContain("file src/a.rs");
    expect(stops.map(name)).not.toContain(`thread ${onA10.threadId}`);
    expect(stops.map(name)[stops.length - 1]).toBe(`thread ${onB5.threadId}`);
  });

  it("keeps each thread's line and side for the comment key", () => {
    const stop = pageStops(input()).find(
      (s) => s.kind === "thread" && s.threadId === onA10.threadId,
    );
    expect(stop).toMatchObject({ card: "src/a.rs", line: 10, side: "new" });
    const listed = pageStops(input()).find(
      (s) => s.kind === "thread" && s.threadId === outdatedA.threadId,
    );
    expect(listed).toMatchObject({ line: 0, side: null });
  });
});

describe("moveFocus", () => {
  const stops = pageStops(input());
  const walk = (delta: 1 | -1, from: FocusItem | null, steps: number) => {
    const seen: string[] = [];
    let at = from;
    for (let i = 0; i < steps; i++) {
      at = moveFocus(stops, at, delta);
      if (!at) break;
      seen.push(name(at));
    }
    return seen;
  };

  it("n walks every thread and finding in page order, then stops", () => {
    const all = walk(1, null, 20);
    expect(all).toEqual(
      stops
        .filter((s) => s.kind === "thread" || s.kind === "finding")
        .map(name),
    );
    expect(moveFocus(stops, stops[stops.length - 1], 1)).toBeNull();
  });

  it("p walks back; from nothing it starts at the bottom", () => {
    expect(walk(-1, null, 2)).toEqual([
      `thread ${onB5.threadId}`,
      `finding ${findB1.id}`,
    ]);
    expect(moveFocus(stops, stops[1], -1)).toBeNull();
  });

  it("from a file goes into that file, back to the one before", () => {
    const fileB: FocusItem = { kind: "file", path: "src/b.rs" };
    expect(name(moveFocus(stops, fileB, 1)!)).toBe(`finding ${findB1.id}`);
    expect(name(moveFocus(stops, fileB, -1)!)).toBe(`finding ${findA10.id}`);
  });

  it("from a clicked line: n the first at or below it, p above it", () => {
    const line: FocusItem = {
      kind: "line",
      path: "src/a.rs",
      side: "new",
      line: 10,
    };
    expect(name(moveFocus(stops, line, 1)!)).toBe(`thread ${onA10.threadId}`);
    expect(name(moveFocus(stops, line, -1)!)).toBe(`thread ${onA3.threadId}`);
  });

  it("from an item no longer shown (a dismissed finding): its line", () => {
    const gone: FocusItem = {
      kind: "finding",
      findingId: "00000000-0000-4000-a000-999999999999",
      card: "src/a.rs",
      line: 5,
      side: "new",
    };
    expect(name(moveFocus(stops, gone, 1)!)).toBe(`thread ${onA10.threadId}`);
    expect(name(moveFocus(stops, gone, -1)!)).toBe(`thread ${onA3.threadId}`);
  });
});

describe("threadDescription", () => {
  it("names the count, state and place", () => {
    expect(threadDescription(onA10)).toBe("1 comment, on src/a.rs line 10");
    expect(threadDescription({ ...mineGeneral, resolved: true })).toBe(
      "1 comment, yours, resolved, on the review as a whole",
    );
  });
});
