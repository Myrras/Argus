/** Fictitious thread histories (M3) for tests. */

import type {
  CodeSnapshot,
  StoredThread,
  ThreadHistory,
  TimelineEntry,
} from "../ipc/types";

export const THEN_SHA = "1111111122222222333333334444444455555555";
export const HEAD_SHA = "a".repeat(40);
export const FIX_SHA = "abcdef0123456789abcdef0123456789abcdef01";

export function snapshot(overrides: Partial<CodeSnapshot> = {}): CodeSnapshot {
  return {
    sha: THEN_SHA,
    path: "src/cache.rs",
    firstLine: 40,
    lines: ["fn get() {", "    let key = raw;", "}"],
    anchorStart: 41,
    anchorEnd: 41,
    source: "file",
    ...overrides,
  };
}

/** A history for `t`: located, untouched, then = now, its notes only. */
export function historyFor(
  t: StoredThread,
  overrides: Partial<ThreadHistory> = {},
): ThreadHistory {
  const notes: TimelineEntry[] = t.notes.map((n, i) => ({
    at: n.createdAt,
    kind: "note",
    noteId: n.id,
    author: n.author,
    body: n.body,
    system: n.system,
    root: i === 0,
  }));
  return {
    threadId: t.threadId,
    reviewId: t.reviewId,
    headSha: HEAD_SHA,
    outdated: t.outdated,
    status: {
      kind: "located",
      path: "src/cache.rs",
      startLine: 41,
      line: 41,
      moved: false,
      renamed: false,
      touched: false,
    },
    touchedBy: [],
    thenSnapshot: snapshot(),
    nowSnapshot: snapshot({ sha: HEAD_SHA }),
    diffLines: [
      {
        kind: "same",
        then: { line: 40, text: "fn get() {", anchored: false },
        now: { line: 40, text: "fn get() {", anchored: false },
      },
      {
        kind: "same",
        then: { line: 41, text: "    let key = raw;", anchored: true },
        now: { line: 41, text: "    let key = raw;", anchored: true },
      },
      {
        kind: "same",
        then: { line: 42, text: "}", anchored: false },
        now: { line: 42, text: "}", anchored: false },
      },
    ],
    timeline: notes,
    ...overrides,
  };
}
