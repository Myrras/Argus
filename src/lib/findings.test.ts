import { describe, expect, it } from "vitest";

import { aiRun, checklistItem, finding } from "../test/aiFixtures";
import {
  aiStatusText,
  currentRound,
  defaultBody,
  findingPlacement,
  fromEarlierRound,
  groupBySeverity,
  sortFindings,
  splitChecklist,
} from "./findings";

describe("findings", () => {
  it("sort high → medium → low, then path and line", () => {
    const list = [
      finding({ severity: "low", path: "a.rs", line: 1, title: "low" }),
      finding({ severity: "high", path: "b.rs", line: 9, title: "high b9" }),
      finding({ severity: "medium", path: "a.rs", line: 5, title: "med" }),
      finding({ severity: "high", path: "b.rs", line: 2, title: "high b2" }),
      finding({ severity: "high", path: "a.rs", line: 7, title: "high a7" }),
    ];
    expect(sortFindings(list).map((f) => f.title)).toEqual([
      "high a7",
      "high b2",
      "high b9",
      "med",
      "low",
    ]);
    expect(
      groupBySeverity(list).map((g) => [g.severity, g.findings.length]),
    ).toEqual([
      ["high", 3],
      ["medium", 1],
      ["low", 1],
    ]);
    expect(groupBySeverity([finding({ severity: "low" })])).toHaveLength(1);
  });

  it("know the current round and mark earlier unposted findings", () => {
    const old = finding({ round: 1 });
    const posted = finding({ round: 1, postedAt: "2026-10-16T11:00:00Z" });
    const now = finding({ round: 2 });
    expect(currentRound(aiRun({ lastDoneRound: 2 }), [old])).toBe(2);
    expect(currentRound(aiRun({ lastDoneRound: null }), [old, now])).toBe(2);
    expect(currentRound(undefined, [])).toBeNull();
    expect(fromEarlierRound(old, 2)).toBe(true);
    expect(fromEarlierRound(posted, 2)).toBe(false);
    expect(fromEarlierRound(now, 2)).toBe(false);
    expect(fromEarlierRound(old, null)).toBe(false);
  });

  it("post as the backend's default without an edit", () => {
    expect(defaultBody(finding({ title: "T", body: "B" }))).toBe("**T**\n\nB");
  });

  it("place a finding on its side, or not when its line is not shown", () => {
    const lines = { a: 10, b: 20 };
    expect(findingPlacement(finding({ line: 12 }), lines, false)).toEqual({
      side: "b",
      line: 12,
    });
    expect(
      findingPlacement(finding({ line: 3, side: "old" }), lines, false),
    ).toEqual({ side: "a", line: 3 });
    expect(findingPlacement(finding({ line: 21 }), lines, false)).toBeNull();
    expect(findingPlacement(finding({ line: 0 }), lines, false)).toBeNull();
    // "Since my last review": the left side is not the base.
    expect(
      findingPlacement(finding({ line: 3, side: "old" }), lines, true),
    ).toBeNull();
  });

  it("split the checklist into its newest round and the earlier ones", () => {
    const a = checklistItem({ round: 1 });
    const b = checklistItem({ round: 3 });
    const c = checklistItem({ round: 2 });
    const d = checklistItem({ round: 3 });
    const split = splitChecklist([a, b, c, d]);
    expect(split.round).toBe(3);
    expect(split.current).toEqual([b, d]);
    expect(split.earlier).toEqual([c, a]);
    expect(splitChecklist([]).round).toBeNull();
  });

  it("word the header chip per run state", () => {
    expect(aiStatusText(aiRun({ status: "running" }), 0)).toBe("AI: running…");
    expect(aiStatusText(aiRun({ status: "done", queued: true }), 2)).toBe(
      "AI: queued",
    );
    expect(aiStatusText(aiRun(), 6)).toBe("AI: done · 6 findings");
    expect(aiStatusText(aiRun(), 1)).toBe("AI: done · 1 finding");
    expect(
      aiStatusText(aiRun({ status: "failed", error: "agent timed out" }), 0),
    ).toBe("AI: failed — agent timed out");
    expect(
      aiStatusText(aiRun({ status: "failed", error: "cancelled" }), 0),
    ).toBe("AI: cancelled");
    expect(aiStatusText(aiRun({ status: "notStarted" }), 0)).toBe(
      "AI: not started",
    );
  });
});
