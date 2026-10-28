import { describe, expect, it } from "vitest";

import { previewText } from "./commentText";

describe("previewText", () => {
  it("drops the markup and labels code", () => {
    expect(
      previewText(
        "## Nit\n- [x] **done** see [the docs](https://example.test/d)\n\n```jq\n.a | .b\n```\nthen `x`",
      ),
    ).toBe("Nit done see the docs [code] then x");
  });

  it("says when a comment suggests a change", () => {
    expect(previewText("Try this:\n```suggestion\nlet x = 1;\n```")).toBe(
      "Try this: [suggested change]",
    );
  });

  it("flattens tables", () => {
    expect(
      previewText("| Case | Before |\n| --- | --- |\n| warm | 95 ms |"),
    ).toBe("Case Before warm 95 ms");
  });

  it("cuts long text", () => {
    expect(previewText("a ".repeat(100), 10)).toBe("a a a a a…");
  });
});
