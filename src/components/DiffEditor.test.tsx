/**
 * Smoke test of the CodeMirror wrapper in jsdom: it builds both layouts,
 * marks the change and puts a thread under its line. Layout-dependent
 * behaviour (measuring, scrolling) is out of reach here.
 */

import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { thread } from "../test/fixtures";
import { DiffEditor } from "./DiffEditor";

// jsdom has no layout: CodeMirror asks ranges for their rectangles.
if (!Range.prototype.getClientRects) {
  Range.prototype.getClientRects = () =>
    ({
      length: 0,
      item: () => null,
      [Symbol.iterator]: [][Symbol.iterator],
    }) as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
}

const original = "fn a() {}\nfn b() {}\nfn c() {}\n";
const modified = "fn a() {}\nfn b2() {}\nfn c() {}\n";

describe("DiffEditor", () => {
  it.each(["unified", "split"] as const)(
    "renders the %s diff with a thread under its line",
    async (layout) => {
      const t = thread({ mine: true, author: "rev-fixture" });
      const { container } = render(
        <DiffEditor
          path="src/lib.rs"
          original={original}
          modified={modified}
          layout={layout}
          placements={[{ thread: t, side: "b", line: 2 }]}
          renderThread={(th) => <p>thread {th.author}</p>}
        />,
      );
      expect(container.querySelectorAll(".cm-editor")).toHaveLength(
        layout === "split" ? 2 : 1,
      );
      expect(container.querySelector(".cm-changedLine")).not.toBeNull();
      expect(await screen.findByText("thread rev-fixture")).toBeInTheDocument();
      // The commenting hook is there and disabled.
      await waitFor(() =>
        expect(container.querySelector(".cm-argusCommentPlus")).toHaveAttribute(
          "title",
          "Commenting comes in a later release",
        ),
      );
    },
  );
});
