/**
 * Smoke test of the CodeMirror wrapper in jsdom: it builds both layouts,
 * marks the change, puts a thread and an item under their lines, and shows
 * the comment "+" on commentable lines only. Layout-dependent behaviour
 * (measuring, scrolling, code clicks) is out of reach here.
 */

import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { useEffect, useState } from "react";
import { describe, expect, it, vi } from "vitest";

import type { StoredThread } from "../ipc/types";
import type { Placement } from "../lib/diffThreads";

import { EditorView } from "@codemirror/view";

import { diffEditorFor, threadBlockKey } from "../lib/diffEditors";
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
    // code's own ↑/↓ + Enter and the Comment-on-line key.
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
  describe("block lifecycle (real CodeMirror)", () => {
    /** Counts mounts and unmounts of what a block renders. */
    function probe() {
      const counts = { mounted: 0, unmounted: 0 };
      function Probe({ label }: { label: string }) {
        useEffect(() => {
          counts.mounted += 1;
          return () => {
            counts.unmounted += 1;
          };
        }, []);
        return <p>{label}</p>;
      }
      return { counts, Probe };
    }

    const at = (t: StoredThread, line = 2): Placement[] => [
      { thread: t, side: "b", line },
    ];

    it("keeps a thread mounted while its notes change and blocks join its line", async () => {
      const { counts, Probe } = probe();
      const t = thread({ author: "rev-fixture" });
      const renderThread = (th: StoredThread) => (
        <Probe label={`${th.author}: ${th.notes.length} notes`} />
      );
      const view = (placements: Placement[], items = [] as never[]) => (
        <DiffEditor
          path="src/lib.rs"
          original={original}
          modified={modified}
          layout="unified"
          placements={placements}
          renderThread={renderThread}
          items={items}
        />
      );
      const { rerender } = render(view(at(t)));
      expect(await screen.findByText("rev-fixture: 1 notes")).toBeVisible();
      // A reply arrives: the same thread with another note.
      const replied = {
        ...t,
        notes: [...t.notes, { ...t.notes[0], id: "n2" }],
      };
      rerender(view(at(replied)));
      expect(await screen.findByText("rev-fixture: 2 notes")).toBeVisible();
      // A pending comment joins the line.
      rerender(
        view(at(replied), [
          {
            key: "draft:1",
            side: "b",
            line: 2,
            render: () => <p>pending draft</p>,
          },
        ] as never[]),
      );
      expect(await screen.findByText("pending draft")).toBeVisible();
      await act(async () => {});
      expect(counts).toEqual({ mounted: 1, unmounted: 0 });
    });

    it("unmounts every block root with the editor", async () => {
      const { counts, Probe } = probe();
      const t = thread();
      const u = thread();
      const { rerender, unmount } = render(
        <DiffEditor
          path="src/lib.rs"
          original={original}
          modified={modified}
          layout="split"
          placements={[...at(t, 1), ...at(u, 3)]}
          renderThread={(th) => <Probe label={th.threadId} />}
        />,
      );
      await screen.findByText(u.threadId);
      // A thread moves to another line: its old block goes.
      rerender(
        <DiffEditor
          path="src/lib.rs"
          original={original}
          modified={modified}
          layout="split"
          placements={[...at(t, 2), ...at(u, 3)]}
          renderThread={(th) => <Probe label={th.threadId} />}
        />,
      );
      await act(async () => {});
      unmount();
      await act(async () => {});
      expect(counts.mounted).toBeGreaterThan(0);
      expect(counts.unmounted).toBe(counts.mounted);
    });

    it("sizes the blocks from the editor's live geometry", async () => {
      const { container } = render(
        <DiffEditor
          path="src/lib.rs"
          original={original}
          modified={modified}
          layout="unified"
          placements={at(thread())}
          renderThread={() => <p>sized</p>}
        />,
      );
      await screen.findByText("sized");
      const block = container.querySelector<HTMLElement>(".cm-argusThreads")!;
      // No fixed pixels from the first measure: CSS variables the editor
      // keeps current as it resizes.
      expect(block.style.width).toBe("");
      const editor = container.querySelector<HTMLElement>(".cm-editor")!;
      await waitFor(() =>
        expect(editor.style.getPropertyValue("--argus-threads-width")).not.toBe(
          "",
        ),
      );
    });
  });
  it("picks a line and comments on it from the keyboard", async () => {
    const onComment = vi.fn();
    function Keyed() {
      const [focus, setFocus] = useState<{
        side: "a" | "b";
        line: number;
      } | null>(null);
      return (
        <DiffEditor
          path="src/lib.rs"
          original={original}
          modified={modified}
          layout="unified"
          placements={[]}
          renderThread={() => null}
          comment={{
            lines: { a: new Set<number>(), b: new Set([2, 3]) },
            selection: null,
            focus,
            onComment,
            onFocusLine: (side, line) => setFocus({ side, line }),
          }}
        />
      );
    }
    const { container } = render(<Keyed />);
    const content = container.querySelector<HTMLElement>(".cm-content")!;
    // A Tab stop with a name that says what the keys do.
    expect(content).toHaveAttribute("tabindex", "0");
    expect(content.getAttribute("aria-label")).toContain("src/lib.rs");
    content.focus();
    fireEvent.keyDown(content, { key: "ArrowDown" });
    await waitFor(() =>
      expect(container.querySelector(".cm-argusFocusedLine")).not.toBeNull(),
    );
    // Line 1 is not commentable: Enter does nothing there, and stays the
    // code view's (it never reaches the list's Enter).
    expect(fireEvent.keyDown(content, { key: "Enter" })).toBe(false);
    expect(onComment).not.toHaveBeenCalled();
    fireEvent.keyDown(content, { key: "ArrowDown" });
    await waitFor(() =>
      expect(container.querySelector(".cm-argusFocusedLine")?.textContent).toBe(
        "fn b2() {}",
      ),
    );
    const enter = fireEvent.keyDown(content, { key: "Enter" });
    expect(enter).toBe(false);
    expect(onComment).toHaveBeenCalledWith("b", 2, 2, false);
  });
});

describe("DiffEditor: revealing a block", () => {
  // Line 2 ends at 19 in `original` ("fn b() {}") and at 20 in `modified`
  // ("fn b2() {}"): the position tells the side apart.
  const scrolledTo = () => {
    const scroll = vi.spyOn(EditorView, "scrollIntoView");
    const dispatch = vi.spyOn(EditorView.prototype, "dispatch");
    return {
      pos: () => scroll.mock.calls[scroll.mock.calls.length - 1]?.[0],
      doc: () => {
        const effect =
          scroll.mock.results[scroll.mock.results.length - 1]?.value;
        const i = dispatch.mock.calls.findIndex((c) => {
          const spec = c[0] as { effects?: unknown } | undefined;
          return spec?.effects === effect;
        });
        return (
          dispatch.mock.contexts[i] as EditorView | undefined
        )?.state.doc.toString();
      },
    };
  };

  it.each([
    ["split", "a", 19, original],
    ["split", "b", 20, modified],
    ["unified", "b", 20, modified],
    // Unified: an old-side block sits where that line shows on the head.
    ["unified", "a", 20, modified],
  ] as const)(
    "%s: scrolls the %s side's editor to the block's line",
    async (layout, side, pos, doc) => {
      const t = thread({ author: "rev-fixture" });
      const spy = scrolledTo();
      render(
        <DiffEditor
          path="src/reveal.rs"
          original={original}
          modified={modified}
          layout={layout}
          placements={[{ thread: t, side, line: 2 }]}
          renderThread={() => <p>thread</p>}
        />,
      );
      await screen.findByText("thread");
      const editor = diffEditorFor("src/reveal.rs");
      expect(editor).not.toBeNull();
      expect(editor!.reveal(threadBlockKey(t.threadId))).toBe(true);
      expect(spy.pos()).toBe(pos);
      expect(spy.doc()).toBe(doc);
    },
  );

  it("reveals an item by its key and refuses an unknown block", async () => {
    render(
      <DiffEditor
        path="src/items.rs"
        original={original}
        modified={modified}
        layout="unified"
        placements={[]}
        renderThread={() => null}
        items={[
          {
            key: "finding:f1",
            side: "b",
            line: 3,
            render: () => <p>finding</p>,
          },
        ]}
      />,
    );
    await screen.findByText("finding");
    const editor = diffEditorFor("src/items.rs")!;
    expect(editor.reveal("finding:f1")).toBe(true);
    expect(editor.reveal("t:missing")).toBe(false);
  });

  it("is gone from the registry once unmounted", async () => {
    const { unmount } = render(
      <DiffEditor
        path="src/gone.rs"
        original={original}
        modified={modified}
        layout="unified"
        placements={[]}
        renderThread={() => null}
      />,
    );
    await waitFor(() => expect(diffEditorFor("src/gone.rs")).not.toBeNull());
    unmount();
    expect(diffEditorFor("src/gone.rs")).toBeNull();
  });
});
