/**
 * Smoke test of the CodeMirror wrapper in jsdom: it builds both layouts,
 * marks the change, puts a thread and an item under their lines, and shows
 * the comment "+" on commentable lines only. Layout-dependent behaviour
 * (measuring, scrolling, code clicks) is out of reach here.
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

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
      // No commenting given: no "+" anywhere.
      expect(container.querySelector(".cm-argusCommentPlus")).toBeNull();
    },
  );

  it("shows an item under its line", async () => {
    render(
      <DiffEditor
        path="src/lib.rs"
        original={original}
        modified={modified}
        layout="unified"
        placements={[]}
        renderThread={() => null}
        items={[
          {
            key: "draft:1",
            side: "b",
            line: 3,
            render: () => <p>pending draft</p>,
          },
        ]}
      />,
    );
    expect(await screen.findByText("pending draft")).toBeInTheDocument();
  });

  it("puts the + on commentable lines only and reports clicks", async () => {
    const onComment = vi.fn();
    const comment = {
      lines: { a: new Set<number>(), b: new Set([2, 3]) },
      selection: null,
      focus: null,
      onComment,
      onFocusLine: vi.fn(),
    };
    const { container } = render(
      <DiffEditor
        path="src/lib.rs"
        original={original}
        modified={modified}
        layout="unified"
        placements={[]}
        renderThread={() => null}
        comment={comment}
      />,
    );
    await waitFor(() =>
      expect(container.querySelectorAll(".cm-argusCommentPlus")).toHaveLength(
        2,
      ),
    );
    // The gutter is aria-hidden (CodeMirror's); the keyboard path is the
    // Comment-on-line key.
    const at = (n: number) =>
      container.querySelector<HTMLElement>(`[data-comment-line="${n}"]`)!;
    expect(at(1)).toBeNull();
    const plus = at(2);
    // A click: mousedown and mouseup on the same "+".
    fireEvent.mouseDown(plus, { button: 0 });
    fireEvent.mouseUp(plus, { button: 0 });
    expect(onComment).toHaveBeenLastCalledWith("b", 2, 2, false);
    // A drag from line 2 to line 3.
    fireEvent.mouseDown(plus, { button: 0 });
    fireEvent.mouseUp(at(3), { button: 0 });
    expect(onComment).toHaveBeenLastCalledWith("b", 2, 3, false);
    // Shift-click extends.
    fireEvent.mouseDown(at(3), { button: 0, shiftKey: true });
    expect(onComment).toHaveBeenLastCalledWith("b", 3, 3, true);
    expect(onComment).toHaveBeenCalledTimes(3);
  });

  it("marks the selected lines", async () => {
    const { container } = render(
      <DiffEditor
        path="src/lib.rs"
        original={original}
        modified={modified}
        layout="unified"
        placements={[]}
        renderThread={() => null}
        comment={{
          lines: { a: new Set<number>(), b: new Set([1, 2, 3]) },
          selection: { side: "b", from: 1, to: 2 },
          focus: null,
          onComment: vi.fn(),
          onFocusLine: vi.fn(),
        }}
      />,
    );
    await waitFor(() =>
      expect(container.querySelectorAll(".cm-argusSelectedLine")).toHaveLength(
        2,
      ),
    );
  });
});
