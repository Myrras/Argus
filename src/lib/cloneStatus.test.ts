import { describe, expect, it } from "vitest";

import type { CloneStatus } from "../ipc/types";
import { cloneTitle, headLine, shortPath } from "./cloneStatus";

const found: CloneStatus = {
  configured: true,
  path: "/Users/fixture/work/widgets",
  found: true,
  head: "c".repeat(40),
  matchesReviewHead: true,
};

describe("shortPath", () => {
  it("shows the home folder as ~ and keeps the last part", () => {
    expect(shortPath("/Users/fixture/vcs/acme/widgets")).toBe("~/…/widgets");
    expect(shortPath("/home/fixture/src/acme/widgets/")).toBe("~/…/widgets");
    expect(shortPath("C:\\Users\\fixture\\vcs\\acme\\widgets")).toBe(
      "~/…/widgets",
    );
  });

  it("keeps a short path whole", () => {
    expect(shortPath("/Users/fixture/widgets")).toBe("~/widgets");
    expect(shortPath("/Users/fixture/work/widgets")).toBe("~/work/widgets");
    expect(shortPath("/srv")).toBe("/srv");
  });

  it("cuts a path outside the home folder to its last part", () => {
    expect(shortPath("/srv/git/acme/widgets")).toBe("/…/widgets");
  });
});

describe("headLine", () => {
  it("says whether the clone is at the review's head", () => {
    expect(headLine(found)).toBe("HEAD ccccccc: at the review's head");
    expect(headLine({ ...found, matchesReviewHead: false })).toBe(
      "HEAD ccccccc: on another commit (Argus reads objects only)",
    );
    expect(headLine({ ...found, head: null })).toBe("HEAD unknown");
  });
});

describe("cloneTitle", () => {
  it("has the full path and the head for a found clone", () => {
    const title = cloneTitle(found);
    expect(title).toContain("/Users/fixture/work/widgets");
    expect(title).toContain("at the review's head");
  });

  it("names a missing clone's path", () => {
    expect(cloneTitle({ ...found, found: false, head: null })).toContain(
      "Local clone not found: /Users/fixture/work/widgets",
    );
  });
});
