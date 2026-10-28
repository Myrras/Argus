import { describe, expect, it } from "vitest";

import { age } from "./time";

const NOW = Date.parse("2026-10-17T00:00:00Z");
const DAY = 86_400_000;
const ago = (days: number) => new Date(NOW - days * DAY).toISOString();

describe("age", () => {
  it("counts weeks up to a full year, never 0y", () => {
    expect(age(ago(13), NOW)).toBe("1w");
    expect(age(ago(364.5), NOW)).toBe("52w");
    expect(age(ago(365), NOW)).toBe("1y");
    expect(age(ago(800), NOW)).toBe("2y");
  });
});
