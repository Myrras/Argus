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
import { afterEach, describe, expect, it, vi } from "vitest";

import type { StoredThread } from "../ipc/types";
import type { Placement } from "../lib/diffThreads";

import { BlockType, EditorView, type WidgetType } from "@codemirror/view";

import { diffEditorFor, threadBlockKey } from "../lib/diffEditors";
import { useLayoutStore } from "../state/layoutStore";
import { thread } from "../test/fixtures";
import { dragLabel, dragReach, dragStop } from "../lib/commentDrag";
import { DiffEditor, type CommentGutter } from "./DiffEditor";

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

afterEach(() => {
  vi.restoreAllMocks();
});

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

  /**
   * jsdom has no layout: the line under the pointer comes from
   * `elementAtHeight`, pointed at a line here.
   */
  function pointAt(line: number | "widget") {
    return vi
      .spyOn(EditorView.prototype, "elementAtHeight")
      .mockImplementation(function (this: EditorView) {
        if (line === "widget") {
          return { from: 0, type: BlockType.WidgetAfter } as never;
        }
        return {
          from: this.state.doc.line(line).from,
          type: BlockType.Text,
        } as never;
      });
  }

  function commentable(
    onComment = vi.fn(),
    regionOf?: CommentGutter["regionOf"],
  ) {
    return {
      lines: { a: new Set<number>(), b: new Set([2, 3]) },
      selection: null,
      focus: null,
      onComment,
      onFocusLine: vi.fn(),
      regionOf,
    };
  }

  it("shows one + beside the commentable line under the pointer", async () => {
    const { container } = render(
      <DiffEditor
        path="src/lib.rs"
        original={original}
        modified={modified}
        layout="unified"
        placements={[]}
        renderThread={() => null}
        comment={commentable()}
      />,
    );
    const editor = container.querySelector<HTMLElement>(".cm-editor")!;
    const plus = () =>
      container.querySelectorAll<HTMLElement>(".cm-argusCommentPlus");
    // Nothing until the pointer is over a line.
    await act(async () => {});
    expect(plus()).toHaveLength(0);
    const spy = pointAt(2);
    fireEvent.mouseMove(editor, { clientY: 10 });
    await waitFor(() => expect(plus()).toHaveLength(1));
    expect(plus()[0].dataset.commentLine).toBe("2");
    expect(plus()[0]).toHaveAccessibleName("Comment on line 2");
    expect(plus()[0].title).toContain("Click to comment, drag to select lines");
    // It follows the pointer.
    spy.mockRestore();
    const at3 = pointAt(3);
    fireEvent.mouseMove(editor, { clientY: 20 });
    await waitFor(() => expect(plus()[0]?.dataset.commentLine).toBe("3"));
    expect(plus()).toHaveLength(1);
    at3.mockRestore();
    // Line 1 takes no comment; a block widget is no line.
    const at1 = pointAt(1);
    fireEvent.mouseMove(editor, { clientY: 0 });
    await waitFor(() => expect(plus()).toHaveLength(0));
    at1.mockRestore();
    const widget = pointAt("widget");
    fireEvent.mouseMove(editor, { clientY: 0 });
    await waitFor(() => expect(plus()).toHaveLength(0));
    widget.mockRestore();
    // Gone when the pointer leaves the editor.
    const again = pointAt(2);
    fireEvent.mouseMove(editor, { clientY: 10 });
    await waitFor(() => expect(plus()).toHaveLength(1));
    fireEvent.mouseLeave(editor);
    await waitFor(() => expect(plus()).toHaveLength(0));
    again.mockRestore();
  });

  it("clicks, drags and shift-clicks from the +", async () => {
    const onComment = vi.fn();
    const { container } = render(
      <DiffEditor
        path="src/lib.rs"
        original={original}
        modified={modified}
        layout="unified"
        placements={[]}
        renderThread={() => null}
        comment={commentable(onComment)}
      />,
    );
    const editor = container.querySelector<HTMLElement>(".cm-editor")!;
    const spy = pointAt(2);
    fireEvent.mouseMove(editor, { clientY: 10 });
    const plus = await waitFor(() => {
      const el = container.querySelector<HTMLElement>(".cm-argusCommentPlus");
      expect(el).not.toBeNull();
      return el!;
    });
    // A click: mousedown and mouseup without a move.
    fireEvent.mouseDown(plus, { button: 0 });
    fireEvent.mouseUp(window, { button: 0 });
    expect(onComment).toHaveBeenLastCalledWith("b", 2, 2, false);
    // A drag from line 2 to line 3: the range shows with a label.
    fireEvent.mouseDown(plus, { button: 0, clientX: 5, clientY: 10 });
    spy.mockRestore();
    const at3 = pointAt(3);
    fireEvent.mouseMove(window, { clientX: 5, clientY: 20 });
    await waitFor(() =>
      expect(container.querySelectorAll(".cm-argusDragLine")).toHaveLength(2),
    );
    expect(container.querySelector(".cm-argusDragStart")?.textContent).toBe(
      "fn b2() {}",
    );
    expect(container.querySelector(".cm-argusDragEnd")?.textContent).toBe(
      "fn c() {}",
    );
    const label = document.querySelector(".argus-comment-drag-label");
    expect(label?.textContent).toBe("Comment on lines 2–3");
    expect(label).toHaveAttribute("role", "status");
    expect(document.documentElement.dataset.commentDrag).toBe("ok");
    fireEvent.mouseUp(window, { button: 0 });
    expect(onComment).toHaveBeenLastCalledWith("b", 2, 3, false);
    // The drag's traces are gone.
    expect(document.querySelector(".argus-comment-drag-label")).toBeNull();
    expect(document.documentElement.dataset.commentDrag).toBeUndefined();
    await waitFor(() =>
      expect(container.querySelector(".cm-argusDragLine")).toBeNull(),
    );
    at3.mockRestore();
    // Shift-click extends.
    pointAt(2);
    fireEvent.mouseMove(editor, { clientY: 10 });
    const again = await waitFor(() => {
      const el = container.querySelector<HTMLElement>(".cm-argusCommentPlus");
      expect(el).not.toBeNull();
      return el!;
    });
    fireEvent.mouseDown(again, { button: 0, shiftKey: true });
    expect(onComment).toHaveBeenLastCalledWith("b", 2, 2, true);
    expect(onComment).toHaveBeenCalledTimes(3);
  });

  it("stops a drag at its region's edge and cancels it with Escape", async () => {
    const onComment = vi.fn();
    // Lines 2 and 3 are two regions (say a hunk and code outside it).
    const regionOf = (_side: "a" | "b", line: number) =>
      line === 2
        ? { key: "h0", outside: false }
        : line === 3
          ? { key: "g1", outside: true }
          : null;
    const { container } = render(
      <DiffEditor
        path="src/lib.rs"
        original={original}
        modified={modified}
        layout="unified"
        placements={[]}
        renderThread={() => null}
        comment={commentable(onComment, regionOf)}
      />,
    );
    const editor = container.querySelector<HTMLElement>(".cm-editor")!;
    const spy = pointAt(2);
    fireEvent.mouseMove(editor, { clientY: 10 });
    const plus = await waitFor(() => {
      const el = container.querySelector<HTMLElement>(".cm-argusCommentPlus");
      expect(el).not.toBeNull();
      return el!;
    });
    fireEvent.mouseDown(plus, { button: 0 });
    spy.mockRestore();
    pointAt(3);
    fireEvent.mouseMove(window, { clientY: 20 });
    await waitFor(() =>
      expect(container.querySelector(".cm-argusDragBlocked")?.textContent).toBe(
        "fn c() {}",
      ),
    );
    expect(container.querySelectorAll(".cm-argusDragLine")).toHaveLength(1);
    expect(document.documentElement.dataset.commentDrag).toBe("blocked");
    expect(
      document.querySelector(".argus-comment-drag-label")?.textContent,
    ).toBe("Comment on line 2 · a range in the diff can't leave its hunk");
    // Escape: nothing is commented on, and a later mouseup does nothing.
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.mouseUp(window, { button: 0 });
    expect(onComment).not.toHaveBeenCalled();
    expect(document.querySelector(".argus-comment-drag-label")).toBeNull();
    expect(document.documentElement.dataset.commentDrag).toBeUndefined();
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

describe("DiffEditor: block heights across a re-placement", () => {
  /** A ResizeObserver double: `fire` reports a height for an element. */
  function fakeResizeObserver() {
    const observers: { cb: ResizeObserverCallback; els: Set<Element> }[] = [];
    class FakeRO {
      private entry: { cb: ResizeObserverCallback; els: Set<Element> };
      constructor(cb: ResizeObserverCallback) {
        this.entry = { cb, els: new Set() };
        observers.push(this.entry);
      }
      observe(el: Element) {
        this.entry.els.add(el);
      }
      unobserve(el: Element) {
        this.entry.els.delete(el);
      }
      disconnect() {
        this.entry.els.clear();
      }
    }
    vi.stubGlobal("ResizeObserver", FakeRO);
    return {
      fire(el: Element, height: number) {
        for (const o of observers) {
          if (!o.els.has(el)) continue;
          o.cb(
            [
              {
                target: el,
                borderBoxSize: [{ blockSize: height, inlineSize: 600 }],
                contentRect: { height } as DOMRectReadOnly,
              } as unknown as ResizeObserverEntry,
            ],
            o as unknown as ResizeObserver,
          );
        }
      },
    };
  }

  /** The thread block widgets the editor decorates with, by line. */
  function widgetsOf(container: HTMLElement): Map<number, WidgetType> {
    const dom = container.querySelector<HTMLElement>(".cm-editor")!;
    const view = EditorView.findFromDOM(dom)!;
    const out = new Map<number, WidgetType>();
    for (const source of view.state.facet(EditorView.decorations)) {
      const set = typeof source === "function" ? source(view) : source;
      set.between(0, view.state.doc.length, (from, _to, deco) => {
        // Ours only, not the merge view's deleted-lines blocks.
        if (
          deco.spec.block &&
          deco.spec.widget &&
          "signature" in deco.spec.widget
        ) {
          out.set(view.state.doc.lineAt(from).number, deco.spec.widget);
        }
      });
    }
    return out;
  }

  it("estimates a rebuilt widget at the height it was measured at", async () => {
    // Every new placement (a finding dismissed, threads re-read) rebuilds the
    // widgets; one above the viewport is not measured again, so a default
    // estimate (one line) would make the page jump.
    const ro = fakeResizeObserver();
    const t = thread({ author: "rev-fixture" });
    const other = thread({ author: "bob-fixture" });
    const props = {
      path: "src/heights.rs",
      original,
      modified,
      layout: "unified" as const,
      renderThread: (th: StoredThread) => <p>thread {th.author}</p>,
    };
    const { container, rerender } = render(
      <DiffEditor
        {...props}
        placements={[{ thread: t, side: "b", line: 2 }]}
      />,
    );
    await screen.findByText("thread rev-fixture");
    ro.fire(container.querySelector(".cm-argusThreads")!, 137);
    rerender(
      <DiffEditor
        {...props}
        placements={[
          { thread: t, side: "b", line: 2 },
          { thread: other, side: "b", line: 3 },
        ]}
      />,
    );
    const widgets = widgetsOf(container);
    expect(widgets.get(2)?.estimatedHeight).toBe(137);
    // A block never measured keeps CodeMirror's default.
    expect(widgets.get(3)?.estimatedHeight).toBe(-1);
    vi.unstubAllGlobals();
  });
});

describe("DiffEditor: switching the layout", () => {
  it("keeps the line under the page's top edge in place", async () => {
    // The page column (the scroller) with its toolbar; the head's line 3
    // sits 140px under the column's top, under the toolbar and header.
    const column = document.createElement("div");
    column.dataset.slot = "diff";
    const bar = document.createElement("div");
    Object.defineProperty(bar, "offsetHeight", { value: 38 });
    column.appendChild(bar);
    document.body.appendChild(column);
    column.getBoundingClientRect = () =>
      ({ top: 0, bottom: 800, height: 800 }) as DOMRect;
    vi.spyOn(EditorView.prototype, "documentTop", "get").mockReturnValue(-60);
    vi.spyOn(EditorView.prototype, "contentHeight", "get").mockReturnValue(
      2000,
    );
    vi.spyOn(EditorView.prototype, "lineBlockAtHeight").mockImplementation(
      function (this: EditorView) {
        return { from: this.state.doc.line(3).from, top: 200 } as never;
      },
    );
    const scroll = vi.spyOn(EditorView, "scrollIntoView");
    const props = {
      path: "src/layout.rs",
      original,
      modified,
      placements: [],
      renderThread: () => null,
    };
    const { rerender } = render(<DiffEditor {...props} layout="unified" />, {
      container: column.appendChild(document.createElement("div")),
    });
    expect(scroll).not.toHaveBeenCalled();
    // The setting changes first; the page re-renders the editors after.
    act(() => useLayoutStore.getState().setDiffLayout("split"));
    rerender(<DiffEditor {...props} layout="split" />);
    // Line 3 of the head ("fn c() {}") starts at 21 in `modified`.
    expect(scroll).toHaveBeenCalledWith(21, { y: "start", yMargin: 140 });
    column.remove();
    useLayoutStore.getState().setDiffLayout("unified");
  });

  it("takes the next line when the one at the edge starts above the column", async () => {
    const column = document.createElement("div");
    column.dataset.slot = "diff";
    column.appendChild(document.createElement("div"));
    document.body.appendChild(column);
    column.getBoundingClientRect = () =>
      ({ top: 0, bottom: 800, height: 800 }) as DOMRect;
    vi.spyOn(EditorView.prototype, "documentTop", "get").mockReturnValue(-60);
    vi.spyOn(EditorView.prototype, "contentHeight", "get").mockReturnValue(
      2000,
    );
    // Line 1 and a tall thread under it reach past the edge from above.
    vi.spyOn(EditorView.prototype, "lineBlockAtHeight").mockImplementation(
      function (this: EditorView) {
        const l = this.state.doc.line(1);
        return { from: l.from, to: l.to, top: 0 } as never;
      },
    );
    vi.spyOn(EditorView.prototype, "lineBlockAt").mockImplementation(function (
      this: EditorView,
      pos: number,
    ) {
      const l = this.state.doc.lineAt(pos);
      return { from: l.from, to: l.to, top: 150 } as never;
    });
    const scroll = vi.spyOn(EditorView, "scrollIntoView");
    const props = {
      path: "src/straddle.rs",
      original,
      modified,
      placements: [],
      renderThread: () => null,
    };
    useLayoutStore.getState().setDiffLayout("split");
    const { rerender } = render(<DiffEditor {...props} layout="split" />, {
      container: column.appendChild(document.createElement("div")),
    });
    act(() => useLayoutStore.getState().setDiffLayout("unified"));
    rerender(<DiffEditor {...props} layout="unified" />);
    // Line 2 ("fn b2() {}" starts at 10), 90px under the column's top.
    expect(scroll).toHaveBeenCalledWith(10, { y: "start", yMargin: 90 });
    column.remove();
  });

  it("leaves the page alone on a rebuild that is no layout switch", async () => {
    const scroll = vi.spyOn(EditorView, "scrollIntoView");
    const props = {
      path: "src/rebuilt.rs",
      original,
      placements: [],
      renderThread: () => null,
      layout: "unified" as const,
    };
    const { rerender } = render(<DiffEditor {...props} modified={modified} />);
    rerender(<DiffEditor {...props} modified={`${modified}fn d() {}\n`} />);
    expect(scroll).not.toHaveBeenCalled();
  });
});

describe("DiffEditor: drag rules", () => {
  const region = (line: number) =>
    line >= 1 && line <= 4
      ? { key: "h0", outside: false }
      : line >= 5 && line <= 20
        ? { key: "g1", outside: true }
        : null;

  it("reaches toward the pointer within the anchor's region", () => {
    expect(dragReach(region, 2, 4)).toEqual({ to: 4, blocked: null });
    expect(dragReach(region, 3, 1)).toEqual({ to: 1, blocked: null });
    expect(dragReach(region, 3, 9)).toEqual({ to: 4, blocked: 9 });
    expect(dragReach(region, 8, 2)).toEqual({ to: 5, blocked: 2 });
    expect(dragReach(region, 30, 2)).toEqual({ to: 30, blocked: null });
  });

  it("names the lines and why a range stops", () => {
    expect(dragLabel(4, 2, null)).toBe("Comment on lines 2–4");
    expect(dragLabel(4, 4, null)).toBe("Comment on line 4");
    expect(dragStop(region, 8, 2)).toBe("enterDiff");
    expect(dragStop(region, 2, 8)).toBe("leaveHunk");
    expect(dragStop(region, 2, 30)).toBe("leaveHunk");
    // Same region: only folded lines in between stopped it.
    expect(dragStop(region, 6, 19)).toBe("folded");
    expect(dragLabel(8, 5, "enterDiff")).toBe(
      "Comment on lines 5–8 · a range outside the diff can't enter it",
    );
    expect(dragLabel(6, 9, "folded")).toBe(
      "Comment on lines 6–9 · unfold the lines in between first",
    );
  });
});

describe("DiffEditor: side by side stays aligned", () => {
  const long = (changed: string) =>
    Array.from({ length: 30 }, (_, i) =>
      i === 1 ? changed : `fn line${i + 1}() {}`,
    ).join("\n") + "\n";

  it("opens a collapsed stretch on both sides when one side opens it", async () => {
    const t = thread({ author: "rev-fixture" });
    const { container, rerender } = render(
      <DiffEditor
        path="src/long.rs"
        original={long("fn old() {}")}
        modified={long("fn new() {}")}
        layout="split"
        placements={[]}
        renderThread={() => <p>thread</p>}
      />,
    );
    const editors = () => [...container.querySelectorAll(".cm-editor")];
    const collapsed = () =>
      editors().map((e) => e.querySelectorAll(".cm-collapsedLines").length);
    await waitFor(() => expect(collapsed()).toEqual([1, 1]));
    // A thread on a base-side line inside the stretch: Argus opens that
    // side, and the head side follows.
    rerender(
      <DiffEditor
        path="src/long.rs"
        original={long("fn old() {}")}
        modified={long("fn new() {}")}
        layout="split"
        placements={[{ thread: t, side: "a", line: 20 }]}
        renderThread={() => <p>thread</p>}
      />,
    );
    await screen.findByText("thread");
    await waitFor(() => expect(collapsed()).toEqual([0, 0]));
  });

  it("follows the library's own click on the stretch", async () => {
    const { container } = render(
      <DiffEditor
        path="src/long2.rs"
        original={long("fn old() {}")}
        modified={long("fn new() {}")}
        layout="split"
        placements={[]}
        renderThread={() => null}
      />,
    );
    const editors = () => [...container.querySelectorAll(".cm-editor")];
    const collapsed = () =>
      editors().map((e) => e.querySelectorAll(".cm-collapsedLines").length);
    await waitFor(() => expect(collapsed()).toEqual([1, 1]));
    fireEvent.click(editors()[1].querySelector(".cm-collapsedLines")!);
    await waitFor(() => expect(collapsed()).toEqual([0, 0]));
  });
});

describe("DiffEditor: mounted again", () => {
  const long = (changed: string) =>
    Array.from({ length: 30 }, (_, i) =>
      i === 1 ? changed : `fn line${i + 1}() {}`,
    ).join("\n") + "\n";

  it.each(["unified", "split"] as const)(
    "opens again the stretches the %s diff had open when it unmounted",
    async (layout) => {
      const path = `src/remount-${layout}.rs`;
      const diff = () => (
        <DiffEditor
          path={path}
          original={long("fn old() {}")}
          modified={long("fn new() {}")}
          layout={layout}
          placements={[]}
          renderThread={() => null}
        />
      );
      const first = render(diff());
      const stretches = (root: HTMLElement) =>
        root.querySelectorAll(".cm-collapsedLines").length;
      await waitFor(() =>
        expect(stretches(first.container)).toBeGreaterThan(0),
      );
      const editors = first.container.querySelectorAll(".cm-editor");
      fireEvent.click(
        editors[editors.length - 1].querySelector(".cm-collapsedLines")!,
      );
      await waitFor(() => expect(stretches(first.container)).toBe(0));
      // Scrolled far away: the page unmounts the diff; back again.
      first.unmount();
      const again = render(diff());
      await waitFor(() =>
        expect(again.container.querySelector(".cm-editor")).not.toBeNull(),
      );
      expect(stretches(again.container)).toBe(0);
      // Another file folds as usual.
      const other = render(
        <DiffEditor
          path="src/other-remount.rs"
          original={long("fn old() {}")}
          modified={long("fn new() {}")}
          layout={layout}
          placements={[]}
          renderThread={() => null}
        />,
      );
      await waitFor(() =>
        expect(stretches(other.container)).toBeGreaterThan(0),
      );
    },
  );
});
