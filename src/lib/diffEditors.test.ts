import { describe, expect, it } from "vitest";

import { blockWidth } from "./diffEditors";

describe("blockWidth", () => {
  it("is the visible width less the gutter and the right gap", () => {
    expect(blockWidth(600, 52)).toBe(540);
  });

  it("follows a narrow side of the split view instead of a minimum", () => {
    // A column of 229 px with a 52 px gutter: the blocks stay inside it.
    expect(blockWidth(229, 52)).toBe(169);
  });

  it("never goes negative", () => {
    expect(blockWidth(40, 52)).toBe(0);
  });
});
