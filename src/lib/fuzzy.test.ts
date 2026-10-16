import { describe, expect, it } from "vitest";

import { fuzzyFilter, fuzzyMatch } from "./fuzzy";
import { paletteRows, type PaletteItem } from "./palette";

describe("fuzzyMatch", () => {
  it("matches everything on a blank query", () => {
    expect(fuzzyMatch("Next file", "  ")).toEqual({ score: 0, indices: [] });
  });

  it("matches characters in order, case-insensitively", () => {
    expect(fuzzyMatch("Next file", "nf")?.indices).toEqual([0, 5]);
    expect(fuzzyMatch("Next file", "fn")).toBeNull();
    expect(fuzzyMatch("Next file", "xyz")).toBeNull();
  });

  it("ranks a substring above a scattered match, a word start above both", () => {
    const sub = fuzzyMatch("Refresh", "fre")!.score;
    const scattered = fuzzyMatch("Reply to thread", "rtt")!.score;
    expect(sub).toBeGreaterThan(scattered);
    expect(fuzzyMatch("Refresh", "ref")!.score).toBeGreaterThan(sub);
  });

  it("prefers word starts for scattered characters", () => {
    expect(fuzzyMatch("src/lib/cache.rs", "slc")?.indices).toEqual([0, 4, 8]);
  });

  it("needs every word of the query", () => {
    expect(fuzzyMatch("Go to file…", "go file")).not.toBeNull();
    expect(fuzzyMatch("Go to file…", "go review")).toBeNull();
  });
});

describe("fuzzyFilter", () => {
  it("drops misses and sorts best first (an earlier hit wins)", () => {
    const out = fuzzyFilter(
      ["Previous file", "Next file", "Refresh", "File viewed"],
      "file",
      (s) => s,
    ).map((x) => x.item);
    expect(out).toEqual(["File viewed", "Next file", "Previous file"]);
  });
});

describe("paletteRows", () => {
  const item = (
    section: PaletteItem["section"],
    label: string,
  ): PaletteItem => ({
    id: `${section}:${label}`,
    section,
    label,
    run: () => {},
  });
  const items = [
    item("Commands", "Go to review…"),
    item("Actions", "Next file"),
    item("Actions", "Refresh"),
    item("Reviews", "acme/widgets#1 Fix the cache"),
    item("Files", "src/cache.rs"),
  ];
  const labels = (rows: ReturnType<typeof paletteRows>) =>
    rows.map((r) => r.item.label);

  it("lists commands and actions without a query", () => {
    expect(labels(paletteRows(items, "all", ""))).toEqual([
      "Go to review…",
      "Next file",
      "Refresh",
    ]);
  });

  it("searches reviews and files too once typed, by section", () => {
    expect(labels(paletteRows(items, "all", "cache"))).toEqual([
      "acme/widgets#1 Fix the cache",
      "src/cache.rs",
    ]);
  });

  it("a Go to mode shows only its section", () => {
    expect(labels(paletteRows(items, "reviews", ""))).toEqual([
      "acme/widgets#1 Fix the cache",
    ]);
    expect(labels(paletteRows(items, "files", "rs"))).toEqual(["src/cache.rs"]);
  });
});
