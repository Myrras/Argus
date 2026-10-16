import { describe, expect, it } from "vitest";

import { review, thread } from "../test/fixtures";
import { splitThreads, tabCounts, threadLocation } from "./reviews";
import { age } from "./time";

describe("reviews helpers", () => {
  it("formats thread locations", () => {
    const base = thread().anchor!;
    expect(threadLocation(thread({ anchor: null }), "gitlab")).toBe("MR-level");
    expect(threadLocation(thread({ anchor: null }), "github")).toBe("PR-level");
    expect(threadLocation(thread({ anchor: base }), "github")).toBe(
      "src/cache.rs:42",
    );
    expect(
      threadLocation(thread({ anchor: { ...base, startLine: 40 } }), "github"),
    ).toBe("src/cache.rs:40–42");
    // A comment on a removed line points at the old side.
    expect(
      threadLocation(
        thread({
          anchor: { ...base, side: "old", pathOld: "src/old.rs", lineOld: 9 },
        }),
        "gitlab",
      ),
    ).toBe("src/old.rs:9");
  });

  it("puts the user's threads first, each group oldest first", () => {
    const t1 = thread({ mine: false, createdAt: "2026-10-01T00:00:00Z" });
    const t2 = thread({ mine: true, createdAt: "2026-10-03T00:00:00Z" });
    const t3 = thread({ mine: true, createdAt: "2026-10-02T00:00:00Z" });
    const { mine, others } = splitThreads([t1, t2, t3]);
    expect(mine).toEqual([t3, t2]);
    expect(others).toEqual([t1]);
  });

  it("counts per tab and ignores approved reviews", () => {
    expect(
      tabCounts([
        review({ status: "needsReview" }),
        review({ status: "approved" }),
        review({ status: "closedWithoutApproval" }),
      ]),
    ).toEqual({
      needsReview: 1,
      needsReReview: 0,
      waitingOnAuthor: 0,
      closedWithoutApproval: 1,
    });
  });

  it("formats compact ages", () => {
    const now = Date.parse("2026-10-16T12:00:00Z");
    expect(age("2026-10-16T11:59:30Z", now)).toBe("now");
    expect(age("2026-10-16T11:55:00Z", now)).toBe("5m");
    expect(age("2026-10-16T09:00:00Z", now)).toBe("3h");
    expect(age("2026-10-13T12:00:00Z", now)).toBe("3d");
    expect(age(null, now)).toBe("");
  });
});
