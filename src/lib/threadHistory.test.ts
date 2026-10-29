import { describe, expect, it } from "vitest";

import type { ThreadHistory } from "../ipc/types";
import { thread } from "../test/fixtures";
import { FIX_SHA, historyFor, snapshot } from "../test/historyFixtures";
import {
  changeText,
  isLongBody,
  nowLocation,
  statusLine,
  thenLocation,
  thenNowColumns,
  threadTimeline,
} from "./threadHistory";

const t = thread();
const commit = (sha: string, title: string): ThreadHistory["timeline"][0] => ({
  at: "2026-10-15T12:00:00Z",
  kind: "commit",
  sha,
  title,
  author: "alice-fixture",
  change: { kind: "deleted" },
});

describe("thenNowColumns", () => {
  it("aligns the rows, padding the side a line is missing from", () => {
    const h = historyFor(t, {
      diffLines: [
        {
          kind: "same",
          then: { line: 40, text: "a", anchored: false },
          now: { line: 40, text: "a", anchored: false },
        },
        {
          kind: "changed",
          then: { line: 41, text: "b", anchored: true },
          now: { line: 41, text: "B", anchored: true },
        },
        {
          kind: "removed",
          then: { line: 42, text: "c", anchored: false },
          now: null,
        },
        {
          kind: "added",
          then: null,
          now: { line: 42, text: "d", anchored: false },
        },
      ],
    });
    const { then, now } = thenNowColumns(h);
    expect(then!.map((l) => [l.line, l.kind, l.anchored])).toEqual([
      [40, "same", false],
      [41, "del", true],
      [42, "del", false],
      [null, "pad", false],
    ]);
    expect(now!.map((l) => [l.line, l.kind, l.anchored])).toEqual([
      [40, "same", false],
      [41, "add", true],
      [null, "pad", false],
      [42, "add", false],
    ]);
  });

  it("uses the snapshots alone without diff rows", () => {
    const h = historyFor(t, { diffLines: [], nowSnapshot: null });
    const { then, now } = thenNowColumns(h);
    expect(then!.map((l) => [l.line, l.anchored])).toEqual([
      [40, false],
      [41, true],
      [42, false],
    ]);
    expect(now).toBeNull();
  });

  it("marks the commented lines removed once they are gone", () => {
    const h = historyFor(t, {
      status: { kind: "deleted", path: "src/cache.rs", removedBy: null },
      diffLines: [],
      nowSnapshot: null,
    });
    expect(thenNowColumns(h).then!.map((l) => l.kind)).toEqual([
      "same",
      "del",
      "same",
    ]);
  });

  it("never gives an unlocated thread a now column", () => {
    const h = historyFor(t, {
      status: { kind: "unlocated", reason: "the history was rewritten" },
    });
    const { then, now } = thenNowColumns(h);
    expect(now).toBeNull();
    expect(then).toHaveLength(3);
  });
});

describe("statusLine", () => {
  const located = (
    s: Partial<{
      startLine: number;
      line: number;
      moved: boolean;
      renamed: boolean;
      touched: boolean;
      path: string;
    }>,
    extra: Partial<ThreadHistory> = {},
  ) =>
    historyFor(t, {
      status: {
        kind: "located",
        path: "src/cache.rs",
        startLine: 41,
        line: 41,
        moved: false,
        renamed: false,
        touched: false,
        ...s,
      },
      ...extra,
    });

  it("says untouched, yours or someone else's", () => {
    expect(statusLine(located({}), true)).toEqual({
      tone: "neutral",
      text: "Untouched since your comment",
    });
    expect(statusLine(located({}), false)?.text).toBe(
      "Untouched since the comment",
    );
  });

  it("says where the lines moved and how many commits touched them", () => {
    const h = located(
      { startLine: 51, line: 52, moved: true, touched: true },
      { touchedBy: ["c1", "c2"] },
    );
    expect(statusLine(h, true)).toEqual({
      tone: "accent",
      text: "Moved from L41 to L51–52 · touched by 2 commits",
    });
  });

  it("names a rename", () => {
    const h = located({ path: "src/store.rs", moved: true, renamed: true });
    expect(statusLine(h, true)?.text).toBe(
      "Renamed src/cache.rs → src/store.rs · untouched since your comment",
    );
  });

  it("names the commit that removed the lines or the file", () => {
    const deleted = historyFor(t, {
      status: { kind: "deleted", path: "src/cache.rs", removedBy: FIX_SHA },
      timeline: [commit(FIX_SHA, "Drop the raw key")],
    });
    expect(statusLine(deleted, true)?.text).toBe(
      "The commented lines were removed in abcdef01 Drop the raw key",
    );
    const noCommit = historyFor(t, {
      status: { kind: "deleted", path: "src/cache.rs", removedBy: null },
    });
    expect(statusLine(noCommit, true)?.text).toBe(
      "The commented lines were removed",
    );
    const gone = historyFor(t, {
      status: { kind: "fileGone", path: "src/cache.rs", removedBy: FIX_SHA },
    });
    expect(statusLine(gone, true)?.text).toBe(
      "The file was deleted in abcdef01",
    );
  });

  it("gives the reason it could not locate the lines", () => {
    const h = historyFor(t, {
      status: { kind: "unlocated", reason: "the history was rewritten" },
    });
    expect(statusLine(h, true)).toEqual({
      tone: "warning",
      text: "Could not locate the lines now: the history was rewritten",
    });
  });

  it("has nothing to say without code", () => {
    expect(
      statusLine(historyFor(t, { status: { kind: "notCode" } }), true),
    ).toBeNull();
  });
});

describe("labels", () => {
  it("captions the columns with path and lines", () => {
    const h = historyFor(t, {
      thenSnapshot: snapshot({ anchorStart: 40, anchorEnd: 41 }),
    });
    expect(thenLocation(h)).toBe("src/cache.rs:40–41");
    expect(nowLocation(h)).toBe("src/cache.rs:41");
    expect(
      nowLocation(
        historyFor(t, { status: { kind: "unlocated", reason: "x" } }),
      ),
    ).toBeNull();
  });

  it("words each change and folds long bodies", () => {
    expect(changeText({ kind: "modified" })).toBe("changed the lines");
    expect(changeText({ kind: "renamed", from: "a.rs", to: "b.rs" })).toBe(
      "renamed a.rs → b.rs",
    );
    expect(isLongBody("short")).toBe(false);
    expect(isLongBody("line\n".repeat(20))).toBe(true);
  });
});

describe("threadTimeline", () => {
  const note = (id: string, at: string, body = id) => ({
    id,
    author: "rev-fixture",
    body,
    createdAt: at,
    system: false,
  });

  it("takes the notes from the thread, not from an older history", () => {
    const before = thread({ notes: [note("n1", "2026-10-15T10:00:00Z")] });
    const h = historyFor(before, {
      timeline: [
        ...historyFor(before).timeline,
        commit("c".repeat(40), "Rework the cache"),
        { at: "2026-10-15T13:00:00Z", kind: "resolved", by: "alice-fixture" },
      ],
    });
    // A reply landed after the history was read.
    const now = {
      ...before,
      notes: [...before.notes, note("n2", "2026-10-15T14:00:00Z", "Thanks!")],
    };
    expect(
      threadTimeline(now, h).map((e) =>
        e.kind === "note" ? `${e.noteId}:${e.root}` : e.kind,
      ),
    ).toEqual(["n1:true", "commit", "resolved", "n2:false"]);
    const reply = threadTimeline(now, h)[3];
    expect(reply).toMatchObject({ kind: "note", body: "Thanks!" });
  });

  it("drops a note the history still has but the thread no longer does", () => {
    const h = historyFor(
      thread({
        notes: [
          note("n1", "2026-10-15T10:00:00Z"),
          note("gone", "2026-10-15T11:00:00Z"),
        ],
      }),
    );
    const now = thread({ notes: [note("n1", "2026-10-15T10:00:00Z")] });
    expect(
      threadTimeline(now, h).map((e) => e.kind === "note" && e.noteId),
    ).toEqual(["n1"]);
  });

  it("puts notes first on a tie and undated commits last", () => {
    const now = thread({ notes: [note("n1", "2026-10-15T12:00:00Z")] });
    const h = historyFor(now, {
      timeline: [
        { ...commit("d".repeat(40), "Undated"), at: null },
        commit("c".repeat(40), "Same time"),
      ],
    });
    expect(
      threadTimeline(now, h).map((e) =>
        e.kind === "commit" ? e.title : e.kind,
      ),
    ).toEqual(["note", "Same time", "Undated"]);
  });
});
