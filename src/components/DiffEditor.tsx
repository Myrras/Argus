/**
 * One file's diff in CodeMirror 6 (`@codemirror/merge`): unified
 * (`unifiedMergeView`, the head with the removed lines as blocks) or side by
 * side (`MergeView`, base left, head right). Read-only, syntax-highlighted
 * by file name, unchanged stretches collapsed (click to expand).
 *
 * Threads sit in block widgets under their line, one React root per line
 * (`renderThread`); so do the caller's other line items (`items`: the open
 * composer, pending comments). A collapsed stretch holding one is expanded
 * up front so nothing hides. A line's widget re-renders into its root when
 * its blocks change (`updateDOM`), so a thread keeps its React state (an
 * open reply, the focus) while notes arrive or blocks join its line; the
 * root is unmounted with the widget's DOM. The blocks sit beside the gutter
 * as wide as the visible part of their own editor (a side of the split view
 * included, however narrow), from CSS variables the editor keeps current
 * (`blockGeometry`).
 *
 * Commenting (`comment`): the gutter's "+" shows on the commentable lines
 * only (hover to see it). A click opens a comment on that line, shift-click
 * extends the open one, a drag over the gutter selects a range; the caller
 * clamps a range to one hunk. A click in the code marks the "focused" line
 * the Comment-on-line key (c) comments on. From the keyboard: the code is a
 * Tab stop, ↑/↓ move the focused line, Enter comments on it (Shift+Enter
 * extends the open comment). The unified view shows the head,
 * so its "+" is on new-side lines; old-side lines are commented on in the
 * side-by-side view.
 */

import { useEffect, useRef, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";

import {
  getChunks,
  MergeView,
  uncollapseUnchanged,
  unifiedMergeView,
} from "@codemirror/merge";
import {
  Compartment,
  EditorState,
  RangeSetBuilder,
  StateEffect,
  StateField,
  type Extension,
} from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  gutter,
  GutterMarker,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from "@codemirror/view";

import type { StoredThread } from "../ipc/types";
import { baseExtensions, languageFor } from "../lib/codemirror";
import {
  collapsedRanges,
  mapOldLine,
  type DiffSide,
  type Placement,
} from "../lib/diffThreads";
import {
  blockHeight,
  blockSignature,
  blockWidth,
  registerDiffEditor,
  rememberBlockHeight,
  threadBlockKey,
} from "../lib/diffEditors";
import { pauseScrollAnchor, readingEdge } from "../lib/scrollAnchor";
import { useLayoutStore, type DiffLayout } from "../state/layoutStore";
import { useThemeStore } from "../state/themeStore";

const COLLAPSE = { margin: 3, minSize: 4 };

export type RenderThread = (
  thread: StoredThread,
  onResize: () => void,
) => ReactNode;

/** Something the caller shows under a line (a composer, a pending comment). */
export interface LineItem {
  /** Stable while the item's DOM may be kept (its content reads a store). */
  key: string;
  side: DiffSide;
  line: number;
  render: (onResize: () => void) => ReactNode;
}

/** One block under a line: a thread or a caller's item. */
interface Block {
  key: string;
  render: (onResize: () => void) => ReactNode;
}

/**
 * Each widget DOM's React root, the block keys it shows (and their
 * signature) and the observer that records its height. Keyed by the DOM,
 * not the widget: CodeMirror hands a kept DOM to a new widget instance
 * (`updateDOM`) and destroys whichever instance holds it last.
 */
const roots = new WeakMap<
  HTMLElement,
  {
    root: Root;
    keys: Set<string>;
    signature: string;
    sizes: ResizeObserver | null;
  }
>();

/** The blocks under one line. */
class BlocksWidget extends WidgetType {
  readonly signature: string;

  constructor(readonly blocks: readonly Block[]) {
    super();
    this.signature = blockSignature(blocks.map((b) => b.key));
  }

  /**
   * The height these blocks had when last drawn: every placement rebuilds
   * the widgets, and one above the viewport is not drawn again, so the
   * default (one line) would shrink the page above the reader
   * (`lib/diffEditors`).
   */
  get estimatedHeight(): number {
    return blockHeight(this.signature);
  }

  /**
   * Never equal: the blocks' render closures are new with every placement,
   * so a kept DOM is always re-rendered (`updateDOM`) rather than left
   * showing stale props.
   */
  eq(): boolean {
    return false;
  }

  toDOM(view: EditorView): HTMLElement {
    const dom = document.createElement("div");
    dom.className = "cm-argusThreads";
    const entry = {
      root: createRoot(dom),
      keys: new Set<string>(),
      signature: this.signature,
      sizes: null as ResizeObserver | null,
    };
    if (typeof ResizeObserver !== "undefined") {
      entry.sizes = new ResizeObserver((entries) => {
        for (const e of entries) {
          const h = e.borderBoxSize?.[0]?.blockSize ?? e.contentRect.height;
          rememberBlockHeight(entry.signature, h);
        }
      });
      entry.sizes.observe(dom);
    }
    roots.set(dom, entry);
    this.show(dom, view);
    return dom;
  }

  /**
   * Re-render into a DOM that shows one of these blocks (React keeps their
   * state by key); a DOM of other blocks is left for a fresh one.
   */
  updateDOM(dom: HTMLElement, view: EditorView): boolean {
    const r = roots.get(dom);
    if (!r || !this.blocks.some((b) => r.keys.has(b.key))) return false;
    this.show(dom, view);
    return true;
  }

  private show(dom: HTMLElement, view: EditorView) {
    const r = roots.get(dom)!;
    r.keys = new Set(this.blocks.map((b) => b.key));
    r.signature = this.signature;
    const resize = () => view.requestMeasure();
    r.root.render(
      <>
        {this.blocks.map((b) => (
          <div key={b.key}>{b.render(resize)}</div>
        ))}
      </>,
    );
  }

  destroy(dom: HTMLElement): void {
    const r = roots.get(dom);
    if (!r) return;
    roots.delete(dom);
    r.sizes?.disconnect();
    // Not during CodeMirror's own update (React warns on a sync unmount
    // inside another render).
    queueMicrotask(() => r.root.unmount());
  }

  /** Clicks and keys belong to the blocks' buttons and fields, not the editor. */
  ignoreEvent(): boolean {
    return true;
  }
}

/** A thread's block key: the thread alone, so new notes keep its state. */
function threadKey(t: StoredThread): string {
  return threadBlockKey(t.threadId);
}

/**
 * The blocks' place: beside the gutter (the content scrolls sideways under
 * long lines; the blocks stay put), as wide as the visible part. Measured on
 * every geometry change (a resize, the file tree or a panel toggled, a wider
 * gutter) into CSS variables, so every block follows without a redraw.
 */
const blockGeometry = ViewPlugin.fromClass(
  class {
    constructor(view: EditorView) {
      this.measure(view);
    }

    update(u: ViewUpdate) {
      if (u.geometryChanged || u.viewportChanged) this.measure(u.view);
    }

    measure(view: EditorView) {
      view.requestMeasure({
        key: this,
        read: (v) => {
          const gutters = v.dom.querySelector<HTMLElement>(".cm-gutters");
          const left = gutters?.offsetWidth ?? 0;
          return {
            left,
            width: blockWidth(v.scrollDOM.clientWidth, left),
          };
        },
        write: ({ left, width }, v) => {
          v.dom.style.setProperty("--argus-threads-left", `${left}px`);
          v.dom.style.setProperty("--argus-threads-width", `${width}px`);
        },
      });
    }
  },
);

/** Lines (1-based) with their blocks, for one editor. */
type LineBlocks = Map<number, Block[]>;

const setBlocks = StateEffect.define<LineBlocks>();

const blockField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    for (const e of tr.effects) {
      if (!e.is(setBlocks)) continue;
      const builder = new RangeSetBuilder<Decoration>();
      const doc = tr.state.doc;
      for (const line of [...e.value.keys()].sort((x, y) => x - y)) {
        if (line > doc.lines) continue;
        builder.add(
          doc.line(line).to,
          doc.line(line).to,
          Decoration.widget({
            widget: new BlocksWidget(e.value.get(line) ?? []),
            block: true,
            side: 1,
          }),
        );
      }
      return builder.finish();
    }
    return deco.map(tr.changes);
  },
  provide: (f) => EditorView.decorations.from(f),
});

/** What one editor knows about commenting. */
interface CommentInfo {
  /** The commentable lines of this editor's side. */
  lines: ReadonlySet<number>;
  /** The selected range (the open composer's lines), inclusive. */
  selection: { from: number; to: number } | null;
  /** The line the Comment-on-line key comments on. */
  focus: number | null;
}

const NO_COMMENT: CommentInfo = {
  lines: new Set(),
  selection: null,
  focus: null,
};

const setComment = StateEffect.define<CommentInfo>();

const commentField = StateField.define<CommentInfo>({
  create: () => NO_COMMENT,
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setComment)) return e.value;
    return value;
  },
});

/** The selected range and the focused line, as line classes. */
const commentLines = EditorView.decorations.compute([commentField], (state) => {
  const info = state.field(commentField);
  const doc = state.doc;
  const builder = new RangeSetBuilder<Decoration>();
  const marks = new Map<number, string>();
  if (info.selection) {
    for (let n = info.selection.from; n <= info.selection.to; n++) {
      marks.set(n, "cm-argusSelectedLine");
    }
  }
  if (info.focus !== null && !marks.has(info.focus)) {
    marks.set(info.focus, "cm-argusFocusedLine");
  }
  for (const n of [...marks.keys()].sort((x, y) => x - y)) {
    if (n < 1 || n > doc.lines) continue;
    const at = doc.line(n).from;
    builder.add(at, at, Decoration.line({ class: marks.get(n)! }));
  }
  return builder.finish();
});

/** The gutter's "+" on a commentable line. */
class CommentMarker extends GutterMarker {
  constructor(readonly line: number) {
    super();
  }

  eq(other: CommentMarker): boolean {
    return other.line === this.line;
  }

  toDOM(): Node {
    const el = document.createElement("span");
    el.className = "cm-argusCommentPlus";
    el.textContent = "+";
    el.title = "Comment on this line (shift-click or drag for a range)";
    el.setAttribute("role", "button");
    el.setAttribute("aria-label", `Comment on line ${this.line}`);
    el.dataset.commentLine = String(this.line);
    return el;
  }
}

const spacer = new (class extends GutterMarker {
  toDOM(): Node {
    const el = document.createElement("span");
    el.textContent = "+";
    el.style.visibility = "hidden";
    return el;
  }
})();

const commentGutter: Extension = [
  blockGeometry,
  commentField,
  commentLines,
  gutter({
    class: "cm-argusCommentGutter",
    lineMarker: (view, block) => {
      const n = view.state.doc.lineAt(block.from).number;
      return view.state.field(commentField).lines.has(n)
        ? new CommentMarker(n)
        : null;
    },
    lineMarkerChange: (u) =>
      u.transactions.some((tr) => tr.effects.some((e) => e.is(setComment))),
    initialSpacer: () => spacer,
  }),
  EditorView.theme({
    ".cm-argusCommentGutter .cm-gutterElement": {
      width: "16px",
      textAlign: "center",
      color: "var(--accent)",
    },
    ".cm-argusCommentPlus": {
      display: "block",
      opacity: "0",
      cursor: "pointer",
      fontWeight: "600",
      borderRadius: "var(--radius-sm)",
      transition: "opacity var(--dur-1) var(--ease-out)",
    },
    ".cm-argusCommentGutter .cm-gutterElement:hover .cm-argusCommentPlus": {
      opacity: "1",
      background: "color-mix(in srgb, var(--accent) 18%, transparent)",
    },
    ".cm-argusSelectedLine": {
      boxShadow: "inset 3px 0 0 var(--accent)",
      backgroundColor: "color-mix(in srgb, var(--accent) 12%, transparent)",
    },
    ".cm-argusFocusedLine": {
      boxShadow: "inset 2px 0 0 var(--border-strong)",
    },
    // Keyboard line picking: the code has the focus, its line is marked.
    "&.cm-focused .cm-argusFocusedLine": {
      boxShadow: "inset var(--focus-ring-width) 0 0 var(--focus-ring)",
    },
    "&.cm-focused .cm-content": {
      outline: "var(--focus-ring-width) solid var(--focus-ring)",
      outlineOffset: "calc(-1 * var(--focus-ring-width))",
    },
    ".cm-argusThreads": {
      padding: "2px 0",
      position: "sticky",
      left: "var(--argus-threads-left, 0px)",
      width: "var(--argus-threads-width, 100%)",
    },
  }),
];

/** Commenting from the gutter, for one diff (both sides). */
export interface CommentGutter {
  /** Commentable lines per side (`a` old, `b` new); empty for none. */
  lines: Record<DiffSide, ReadonlySet<number>>;
  /** The open composer's lines on this file. */
  selection: { side: DiffSide; from: number; to: number } | null;
  /** The focused line on this file. */
  focus: { side: DiffSide; line: number } | null;
  /**
   * A gutter click (`anchor` = `line`), drag (`anchor` where it started) or
   * shift-click (`extend`) on a commentable line.
   */
  onComment: (
    side: DiffSide,
    anchor: number,
    line: number,
    extend: boolean,
  ) => void;
  /** A click in the code. */
  onFocusLine: (side: DiffSide, line: number) => void;
}

function infoFor(
  c: CommentGutter | null | undefined,
  side: DiffSide,
): CommentInfo {
  if (!c) return NO_COMMENT;
  return {
    lines: c.lines[side],
    selection:
      c.selection?.side === side
        ? { from: c.selection.from, to: c.selection.to }
        : null,
    focus: c.focus?.side === side ? c.focus.line : null,
  };
}

/**
 * Gutter clicks, shift-clicks and drags, and code clicks, on one editor:
 * native listeners on its DOM (the gutter is outside the content, and a
 * read-only editor has no cursor to read the line from). Returns the
 * cleanup.
 */
function listenForComments(
  view: EditorView,
  side: DiffSide,
  get: () => CommentGutter | null | undefined,
): () => void {
  const lineOf = (target: EventTarget | null): number | null => {
    const el =
      target instanceof Element
        ? target.closest<HTMLElement>("[data-comment-line]")
        : null;
    if (!el || !view.dom.contains(el)) return null;
    const n = Number(el.dataset.commentLine);
    return Number.isInteger(n) && n > 0 ? n : null;
  };
  let dragFrom: number | null = null;
  const onUp = (e: MouseEvent) => {
    window.removeEventListener("mouseup", onUp);
    const from = dragFrom;
    dragFrom = null;
    if (from === null) return;
    get()?.onComment(side, from, lineOf(e.target) ?? from, false);
  };
  const onDown = (e: MouseEvent) => {
    if (e.button !== 0) return;
    const line = lineOf(e.target);
    if (line !== null) {
      e.preventDefault();
      if (e.shiftKey) {
        get()?.onComment(side, line, line, true);
        return;
      }
      dragFrom = line;
      window.addEventListener("mouseup", onUp);
      return;
    }
    if (!(e.target instanceof Node) || !view.contentDOM.contains(e.target)) {
      return;
    }
    // Not in a block widget (a thread or a composer).
    if (e.target instanceof Element && e.target.closest(".cm-argusThreads")) {
      return;
    }
    const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
    if (pos === null) return;
    get()?.onFocusLine(side, view.state.doc.lineAt(pos).number);
  };
  // The keyboard path to the "+": ↑/↓ pick a line, Enter comments on it.
  const onKey = (e: KeyboardEvent) => {
    // Keys in a block's field or button are theirs.
    if (e.target !== view.contentDOM || e.altKey || e.metaKey || e.ctrlKey) {
      return;
    }
    const c = get();
    if (!c) return;
    const doc = view.state.doc;
    const cur = c.focus?.side === side ? c.focus.line : null;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      const step = e.key === "ArrowDown" ? 1 : -1;
      const start = doc.lineAt(view.viewport.from).number;
      const next = Math.min(
        doc.lines,
        Math.max(1, cur === null ? start : cur + step),
      );
      e.preventDefault();
      c.onFocusLine(side, next);
      view.dispatch({
        effects: EditorView.scrollIntoView(doc.line(next).from, {
          y: "nearest",
        }),
      });
      return;
    }
    if (e.key === "Enter") {
      // The code view's own, commentable line or not: it must not fall
      // through to the list's Enter (which opens the review under its
      // cursor).
      e.preventDefault();
      if (cur !== null && c.lines[side].has(cur)) {
        c.onComment(side, cur, cur, e.shiftKey);
      }
    }
  };
  view.dom.addEventListener("mousedown", onDown);
  view.contentDOM.addEventListener("keydown", onKey);
  return () => {
    view.dom.removeEventListener("mousedown", onDown);
    view.contentDOM.removeEventListener("keydown", onKey);
    window.removeEventListener("mouseup", onUp);
  };
}

function group(
  placements: readonly Placement[],
  items: readonly LineItem[],
  side: DiffSide,
  render: RenderThread,
): LineBlocks {
  const out: LineBlocks = new Map();
  const add = (line: number, b: Block) =>
    out.set(line, [...(out.get(line) ?? []), b]);
  for (const p of placements) {
    if (p.side !== side) continue;
    const t = p.thread;
    add(p.line, { key: threadKey(t), render: (r) => render(t, r) });
  }
  for (const i of items) {
    if (i.side === side) add(i.line, { key: i.key, render: i.render });
  }
  return out;
}

/** Where the reader is in a diff: a head line and its offset in the column. */
interface ReadingLine {
  line: number;
  /** From the page column's top to the line's top. */
  offset: number;
}

/**
 * The head line under the page column's reading edge, when that edge is
 * inside this diff; null otherwise. The head is the last view (unified:
 * the only one; split: `b`), so the line numbers carry across layouts.
 */
function readingLine(views: readonly EditorView[]): ReadingLine | null {
  const view = views[views.length - 1];
  const column = view?.dom.closest<HTMLElement>('[data-slot="diff"]');
  if (!view || !column) return null;
  const edge = readingEdge(column);
  const top = view.documentTop;
  if (edge < top || edge > top + view.contentHeight) return null;
  const columnTop = column.getBoundingClientRect().top;
  let block = view.lineBlockAtHeight(edge - top);
  // A line whose blocks (a thread, a finding) straddle the edge starts
  // above the column: take the next line, whose top is in view.
  if (top + block.top < columnTop && block.to < view.state.doc.length) {
    block = view.lineBlockAt(block.to + 1);
  }
  return {
    line: view.state.doc.lineAt(block.from).number,
    offset: Math.max(0, Math.round(top + block.top - columnTop)),
  };
}

/** Frames to hold a restored line while the rebuilt page measures. */
const HOLD_FRAMES = 30;

/**
 * Keep the line at `pos` `offset` px under the page column's top while
 * the rebuilt editors measure their widgets (the heights above it change
 * for a few frames after CodeMirror scrolled there). Stops once it held
 * still for a few frames, after `HOLD_FRAMES`, or when the reader scrolls.
 */
function holdLine(view: EditorView, pos: number, offset: number) {
  const column = view.dom.closest<HTMLElement>('[data-slot="diff"]');
  if (!column || typeof requestAnimationFrame !== "function") return;
  let frames = 0;
  let still = 0;
  const stop = () => {
    column.removeEventListener("wheel", stop);
    column.removeEventListener("keydown", stop);
    frames = HOLD_FRAMES;
  };
  column.addEventListener("wheel", stop, { passive: true });
  column.addEventListener("keydown", stop);
  const tick = () => {
    if (frames++ >= HOLD_FRAMES || !view.dom.isConnected) return stop();
    pauseScrollAnchor(100);
    // Where the line really is (drawn), else where the height map has it.
    const drawn = view.coordsAtPos(pos, 1)?.top;
    const top =
      (drawn ?? view.documentTop + view.lineBlockAt(pos).top) -
      column.getBoundingClientRect().top;
    const delta = top - offset;
    if (Math.abs(delta) >= 1) {
      column.scrollTop += delta;
      still = 0;
    } else if (++still >= 3) {
      return stop();
    }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

/** Each block key's editor index and line. */
function locate(
  perView: readonly LineBlocks[],
): Map<string, { view: number; line: number }> {
  const out = new Map<string, { view: number; line: number }>();
  perView.forEach((lines, view) => {
    for (const [line, blocks] of lines) {
      for (const b of blocks) out.set(b.key, { view, line });
    }
  });
  return out;
}

/** Expand every collapsed stretch of `view` that holds one of `lines`. */
function uncollapseAt(view: EditorView, side: DiffSide, lines: number[]) {
  const info = getChunks(view.state);
  if (!info || lines.length === 0) return;
  const doc = view.state.doc;
  const ranges = collapsedRanges(
    info.chunks,
    doc,
    side,
    COLLAPSE.margin,
    COLLAPSE.minSize,
  );
  const starts = new Set<number>();
  for (const line of lines) {
    if (line > doc.lines) continue;
    const pos = doc.line(line).from;
    const hit = ranges.find((r) => pos >= r.from && pos <= r.to);
    if (hit) starts.add(hit.from);
  }
  if (starts.size) {
    view.dispatch({
      effects: [...starts].map((p) => uncollapseUnchanged.of(p)),
    });
  }
}

export function DiffEditor({
  path,
  original,
  modified,
  layout,
  placements,
  renderThread,
  items = NO_ITEMS,
  comment = null,
}: {
  /** For the language. */
  path: string;
  /** The base side ("" for an added file). */
  original: string;
  /** The head side ("" for a removed file). */
  modified: string;
  layout: DiffLayout;
  placements: readonly Placement[];
  renderThread: RenderThread;
  /** Other blocks under lines (the composer, pending comments). */
  items?: readonly LineItem[];
  /** Commenting from the gutter; null for none. */
  comment?: CommentGutter | null;
}) {
  const host = useRef<HTMLDivElement>(null);
  const mode = useThemeStore((s) => s.mode);
  const views = useRef<EditorView[]>([]);
  // Where each block sits (editor index, line), for `reveal`.
  const where = useRef(new Map<string, { view: number; line: number }>());
  // The line being read when the layout switched: the rebuilt editor puts
  // it back at the same height. Noted as the setting changes, before any
  // editor of the page is torn down (each one gone shifts the others).
  const reading = useRef<ReadingLine | null>(null);
  useEffect(
    () =>
      useLayoutStore.subscribe((s, prev) => {
        if (s.diffLayout !== prev.diffLayout) {
          reading.current = readingLine(views.current);
        }
      }),
    [],
  );
  const theme = useRef(new Compartment());
  const language = useRef(new Compartment());
  // The latest props for the (re)build effect without rebuilding on each.
  const latest = useRef({ placements, renderThread, mode, comment });
  latest.current = { placements, renderThread, mode, comment };

  // Build the editor(s) for this content and layout.
  useEffect(() => {
    const parent = host.current;
    if (!parent) return;
    const { mode: m } = latest.current;
    const common = (): Extension[] => [
      theme.current.of(baseExtensions(m)),
      language.current.of([]),
      blockField,
      commentGutter,
      // A Tab stop for the keyboard line picking.
      EditorView.contentAttributes.of({
        tabindex: "0",
        "aria-label": `${path}: arrow keys pick a line, Enter comments on it`,
      }),
    ];
    let merge: MergeView | null = null;
    let unified: EditorView | null = null;
    if (layout === "split") {
      merge = new MergeView({
        a: { doc: original, extensions: common() },
        b: { doc: modified, extensions: common() },
        parent,
        gutter: true,
        highlightChanges: true,
        collapseUnchanged: COLLAPSE,
      });
      views.current = [merge.a, merge.b];
    } else {
      unified = new EditorView({
        parent,
        state: EditorState.create({
          doc: modified,
          extensions: [
            ...common(),
            unifiedMergeView({
              original,
              mergeControls: false,
              gutter: true,
              highlightChanges: true,
              syntaxHighlightDeletions: true,
              collapseUnchanged: COLLAPSE,
            }),
          ],
        }),
      });
      views.current = [unified];
    }
    const sides: DiffSide[] = layout === "split" ? ["a", "b"] : ["b"];
    const unlisten = views.current.map((v, i) =>
      listenForComments(v, sides[i], () => latest.current.comment),
    );

    let cancelled = false;
    void languageFor(path).then((lang) => {
      if (cancelled || !lang) return;
      for (const v of views.current) {
        v.dispatch({ effects: language.current.reconfigure(lang) });
      }
    });

    return () => {
      cancelled = true;
      for (const off of unlisten) off();
      merge?.destroy();
      unified?.destroy();
      views.current = [];
    };
  }, [path, original, modified, layout]);

  // Place the threads and items (again whenever they change).
  useEffect(() => {
    const vs = views.current;
    if (vs.length === 0) return;
    const { renderThread: render } = latest.current;
    if (vs.length === 2) {
      const [a, b] = vs;
      const onA = group(placements, items, "a", render);
      const onB = group(placements, items, "b", render);
      uncollapseAt(a, "a", [...onA.keys()]);
      uncollapseAt(b, "b", [...onB.keys()]);
      a.dispatch({ effects: setBlocks.of(onA) });
      b.dispatch({ effects: setBlocks.of(onB) });
      where.current = locate([onA, onB]);
    } else {
      // Unified: base-side blocks go where those lines show on the head.
      const [view] = vs;
      const info = getChunks(view.state);
      const lines = group(placements, items, "b", render);
      const onA = group(placements, items, "a", render);
      if (info && onA.size) {
        const origDoc = EditorState.create({ doc: original }).doc;
        for (const [line, blocks] of onA) {
          if (line > origDoc.lines) continue;
          const at = mapOldLine(info.chunks, origDoc, view.state.doc, line);
          lines.set(at, [...(lines.get(at) ?? []), ...blocks]);
        }
      }
      uncollapseAt(view, "b", [...lines.keys()]);
      view.dispatch({ effects: setBlocks.of(lines) });
      where.current = locate([lines]);
    }
    // After a rebuild (a layout switch), back to the line being read once
    // its blocks are placed: every height in the page changed, so only the
    // line itself says where the reader was.
    const keep = reading.current;
    reading.current = null;
    if (keep) {
      const head = vs[vs.length - 1];
      const doc = head.state.doc;
      const at = doc.line(Math.min(keep.line, doc.lines)).from;
      uncollapseAt(head, "b", [keep.line]);
      head.dispatch({
        effects: EditorView.scrollIntoView(at, {
          y: "start",
          yMargin: keep.offset,
        }),
      });
      holdLine(head, at, keep.offset);
    }
  }, [placements, items, original, modified, layout]);

  // A keyboard move to a block far down: CodeMirror renders only lines
  // near the viewport, so scroll there first (`lib/revealItem`). The
  // effect walks up to the page column, the scroller here.
  useEffect(
    () =>
      registerDiffEditor(path, {
        reveal: (key) => {
          const at = where.current.get(key);
          const view = at ? views.current[at.view] : undefined;
          if (!at || !view) return false;
          const doc = view.state.doc;
          const line = doc.line(Math.min(at.line, doc.lines));
          view.dispatch({
            effects: EditorView.scrollIntoView(line.to, {
              y: "start",
              yMargin: 80,
            }),
          });
          return true;
        },
      }),
    [path],
  );

  // The "+" lines, the selection and the focused line.
  useEffect(() => {
    const vs = views.current;
    const sides: DiffSide[] = vs.length === 2 ? ["a", "b"] : ["b"];
    vs.forEach((v, i) => {
      // The unified view is the head: only new-side lines get a "+".
      const info = infoFor(comment, sides[i]);
      v.dispatch({ effects: setComment.of(info) });
      if (info.selection) uncollapseAt(v, sides[i], [info.selection.from]);
      if (info.focus !== null) uncollapseAt(v, sides[i], [info.focus]);
    });
  }, [comment, original, modified, layout]);

  // Theme switches reconfigure in place.
  useEffect(() => {
    for (const v of views.current) {
      v.dispatch({ effects: theme.current.reconfigure(baseExtensions(mode)) });
    }
  }, [mode]);

  return <div ref={host} className="argus-diff-editor" data-layout={layout} />;
}

const NO_ITEMS: readonly LineItem[] = [];
