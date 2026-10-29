/**
 * The find bar's matches in CodeMirror (`lib/findBridge` carries the
 * state): every editor that knows what it shows (`findTarget`) marks the
 * matches of the query in its visible lines, the current one standing
 * out (`cm-findMatch`, `cm-findCurrent`; `--find-match` /
 * `--find-current`). One state for all: `findState` is a StateField each
 * editor creates from the bridge's latest value and updates from its
 * broadcasts, so an editor that mounts later (a file scrolled near, a
 * comment's code block) shows the matches at once.
 *
 * - A diff side (`kind: "diff"`): the head's every line; the base's only
 *   the removed lines (a changed chunk's), as the search counts them
 *   (`lib/findInReview`). The unified view draws the removed lines itself
 *   (`@codemirror/merge`'s deletion blocks, built once per chunk), out of
 *   reach of decorations: their matches are painted with the CSS Custom
 *   Highlight API (`::highlight(argus-find)`), where the webview has it.
 * - A comment's code block (`kind: "text"`): every line, its offsets
 *   counted from where the code starts in the comment's text (`base`).
 *
 * `findKeys`: Mod+F (or the user's binding for `find`) in an editor opens
 * the find bar instead of CodeMirror's own search panel; it runs before
 * any keymap of the editor.
 */

import { getChunks, getOriginalDoc } from "@codemirror/merge";
import {
  Facet,
  Prec,
  RangeSetBuilder,
  StateEffect,
  StateField,
  type EditorState,
  type Extension,
} from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";

import type { Side } from "../ipc/types";
import {
  findHighlightState,
  onFindHighlight,
  openFindFrom,
  type FindHighlightState,
} from "./findBridge";
import { codeUnit } from "./findInReview";
import { lineMatches } from "./findQuery";
import { findAction, matches as matchesBinding } from "./keymap";
import { currentKeymap } from "./useKeymap";

export type FindTarget =
  | {
      kind: "diff";
      path: string;
      side: Side;
      /** The unified view: the head, with the removed lines as blocks. */
      unified: boolean;
    }
  | {
      kind: "text";
      /** The comment's unit (`findInReview.noteUnit` …). */
      unit: string;
      /** Where the code starts in the comment's text. */
      base: number;
    };

/** What this editor shows; without one, nothing is highlighted. */
export const findTarget = Facet.define<FindTarget, FindTarget | null>({
  combine: (values) => values[0] ?? null,
});

export const setFindState = StateEffect.define<FindHighlightState>();

export const findState = StateField.define<FindHighlightState>({
  create: () => findHighlightState(),
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setFindState)) value = e.value;
    return value;
  },
});

const matchMark = Decoration.mark({ class: "cm-findMatch" });
const currentMark = Decoration.mark({ class: "cm-findMatch cm-findCurrent" });

/** The base lines of `state`'s changed chunks (a split view's left side). */
function removedLines(state: EditorState): (line: number) => boolean {
  const chunks = getChunks(state)?.chunks ?? [];
  const doc = state.doc;
  const runs = chunks
    .filter((c) => c.toA > c.fromA)
    .map((c) => [
      doc.lineAt(Math.min(doc.length, c.fromA)).number,
      doc.lineAt(Math.min(doc.length, Math.max(c.fromA, c.toA - 1))).number,
    ]);
  return (line) => runs.some(([from, to]) => line >= from && line <= to);
}

/**
 * The find decorations of `state` within `ranges` (the visible ones), in
 * document order. Pure over the state: the tests run it on an
 * `EditorState`.
 */
export function findDecorations(
  state: EditorState,
  ranges: readonly { from: number; to: number }[],
): DecorationSet {
  const f = state.field(findState, false);
  const target = state.facet(findTarget);
  if (!f?.re || !target) return Decoration.none;
  if (target.kind === "diff" && f.scope === "comments") return Decoration.none;
  if (target.kind === "text" && f.scope === "code") return Decoration.none;
  const doc = state.doc;
  const unit =
    target.kind === "diff" ? codeUnit(target.path, target.side) : target.unit;
  const current = f.current?.unit === unit ? f.current : null;
  const allowed =
    target.kind === "diff" && target.side === "old"
      ? removedLines(state)
      : null;
  const builder = new RangeSetBuilder<Decoration>();
  let last = 0;
  for (const r of ranges) {
    const first = doc.lineAt(r.from).number;
    const end = doc.lineAt(r.to).number;
    for (let n = Math.max(first, last + 1); n <= end; n++) {
      last = n;
      if (allowed && !allowed(n)) continue;
      const line = doc.line(n);
      for (const [s, e] of lineMatches(line.text, f.re)) {
        const isCurrent =
          !!current &&
          (target.kind === "diff"
            ? current.line === n && current.start === s
            : current.start === target.base + line.from + s);
        builder.add(
          line.from + s,
          line.from + e,
          isCurrent ? currentMark : matchMark,
        );
      }
    }
  }
  return builder.finish();
}

// -- The unified view's removed lines: CSS highlights ---------------------------

interface Painted {
  all: AbstractRange[];
  current: AbstractRange[];
}

/** Each unified editor's ranges, merged into the two named highlights. */
const painted = new Map<EditorView, Painted>();

/** Painted lines per editor at most (a one-letter query on a huge block). */
const MAX_PAINTED = 3000;
/** How far outside the window a block is still painted. */
const PAINT_MARGIN_PX = 1200;

interface HighlightRegistry {
  set: (name: string, h: unknown) => void;
  delete: (name: string) => void;
}

function registry(): {
  highlights: HighlightRegistry;
  make: (ranges: AbstractRange[]) => unknown;
} | null {
  const css = (globalThis as { CSS?: { highlights?: HighlightRegistry } }).CSS;
  const Ctor = (
    globalThis as { Highlight?: new (...r: AbstractRange[]) => unknown }
  ).Highlight;
  if (!css?.highlights || !Ctor) return null;
  return {
    highlights: css.highlights,
    make: (ranges) => {
      // Added one by one: a spread of thousands of arguments is slow.
      const h = new Ctor() as { add: (r: AbstractRange) => void };
      for (const r of ranges) h.add(r);
      return h;
    },
  };
}

let repaintFrame = 0;

/** Merge every editor's ranges into the highlights, once per frame. */
function repaint() {
  if (repaintFrame) return;
  const run = () => {
    repaintFrame = 0;
    const reg = registry();
    if (!reg) return;
    const all: AbstractRange[] = [];
    const current: AbstractRange[] = [];
    for (const p of painted.values()) {
      for (const r of p.all) all.push(r);
      for (const r of p.current) current.push(r);
    }
    if (all.length) reg.highlights.set("argus-find", reg.make(all));
    else reg.highlights.delete("argus-find");
    if (current.length) {
      reg.highlights.set("argus-find-current", reg.make(current));
    } else reg.highlights.delete("argus-find-current");
  };
  if (typeof requestAnimationFrame === "function") {
    repaintFrame = requestAnimationFrame(run);
  } else run();
}

/**
 * Static ranges (cheap: the browser does not keep them up to date as the
 * DOM changes, and the deletion blocks never change) for `[start, end)`
 * pairs of the text below `el`, in one walk over its text nodes.
 */
function textRanges(
  el: Element,
  spans: readonly [number, number][],
): (AbstractRange | null)[] {
  const nodes: { node: Text; at: number }[] = [];
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  let at = 0;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    nodes.push({ node: n as Text, at });
    at += n.textContent?.length ?? 0;
  }
  const spot = (offset: number, end: boolean) => {
    for (const x of nodes) {
      const len = x.node.length;
      if (end ? offset <= x.at + len : offset < x.at + len) {
        return { node: x.node, offset: offset - x.at };
      }
    }
    return null;
  };
  const out: (AbstractRange | null)[] = [];
  for (const [s, e] of spans) {
    const a = spot(s, false);
    const b = spot(e, true);
    if (!a || !b) {
      out.push(null);
      continue;
    }
    out.push(
      typeof StaticRange === "function"
        ? new StaticRange({
            startContainer: a.node,
            startOffset: a.offset,
            endContainer: b.node,
            endOffset: b.offset,
          })
        : (() => {
            const r = document.createRange();
            r.setStart(a.node, a.offset);
            r.setEnd(b.node, b.offset);
            return r;
          })(),
    );
  }
  return out;
}

/**
 * Paint the matches in a unified editor's removed-line blocks: the blocks
 * near the window only (CodeMirror draws a block whole, however long; the
 * editor repaints as the page scrolls), at most `MAX_PAINTED` lines.
 */
function paintDeleted(view: EditorView, target: FindTarget) {
  if (!registry()) return;
  const f = view.state.field(findState, false);
  const out: Painted = { all: [], current: [] };
  if (f?.re && f.scope !== "comments" && target.kind === "diff") {
    const chunks = getChunks(view.state)?.chunks ?? [];
    const orig = getOriginalDoc(view.state);
    const unit = codeUnit(target.path, "old");
    const cur = f.current?.unit === unit ? f.current : null;
    const top = -PAINT_MARGIN_PX;
    const bottom = window.innerHeight + PAINT_MARGIN_PX;
    let lines = 0;
    for (const block of view.dom.querySelectorAll(".cm-deletedChunk")) {
      const box = block.getBoundingClientRect();
      if (box.bottom < top || box.top > bottom) continue;
      let pos: number;
      try {
        pos = view.posAtDOM(block);
      } catch {
        continue;
      }
      const chunk = chunks.find((c) => c.fromB === pos && c.toA > c.fromA);
      if (!chunk) continue;
      const first = orig.lineAt(chunk.fromA).number;
      const rows = block.querySelectorAll(".cm-deletedLine");
      for (let k = 0; k < rows.length && lines < MAX_PAINTED; k++) {
        const el = rows[k];
        const r = el.getBoundingClientRect();
        if (r.bottom < top || r.top > bottom) continue;
        lines++;
        const spans = lineMatches(el.textContent ?? "", f.re);
        if (spans.length === 0) continue;
        const n = first + k;
        const ranges = textRanges(el, spans);
        ranges.forEach((range, i) => {
          if (!range) return;
          if (cur && cur.line === n && cur.start === spans[i][0]) {
            out.current.push(range);
          } else out.all.push(range);
        });
      }
    }
  }
  const had = painted.has(view);
  if (out.all.length || out.current.length) painted.set(view, out);
  else painted.delete(view);
  if (had || painted.has(view)) repaint();
}

// -- The plugin ---------------------------------------------------------------------

/** Every live editor with the extension: the bridge's broadcasts reach them. */
const views = new Set<EditorView>();
let unsubscribe: (() => void) | null = null;

function listen() {
  unsubscribe ??= onFindHighlight((s) => {
    for (const v of views) v.dispatch({ effects: setFindState.of(s) });
  });
}

const findMarks = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    frame = 0;

    constructor(readonly view: EditorView) {
      views.add(view);
      listen();
      // Created between a broadcast and its dispatch: catch up.
      const now = findHighlightState();
      if (view.state.field(findState, false) !== now) {
        queueMicrotask(() => {
          if (views.has(view)) view.dispatch({ effects: setFindState.of(now) });
        });
      }
      this.decorations = findDecorations(view.state, view.visibleRanges);
      this.schedulePaint();
    }

    update(u: ViewUpdate) {
      const changed =
        u.startState.field(findState, false) !==
        u.state.field(findState, false);
      if (changed || u.docChanged || u.viewportChanged) {
        this.decorations = findDecorations(u.state, u.view.visibleRanges);
      }
      if (changed || u.viewportChanged || u.heightChanged) this.schedulePaint();
    }

    /** After the deletion blocks are drawn (the unified view only). */
    schedulePaint() {
      const target = this.view.state.facet(findTarget);
      if (target?.kind !== "diff" || !target.unified) return;
      if (typeof requestAnimationFrame !== "function") return;
      cancelAnimationFrame(this.frame);
      this.frame = requestAnimationFrame(() => paintDeleted(this.view, target));
    }

    destroy() {
      views.delete(this.view);
      cancelAnimationFrame(this.frame);
      if (painted.delete(this.view)) repaint();
    }
  },
  { decorations: (p) => p.decorations },
);

const findTheme = EditorView.baseTheme({
  ".cm-findMatch": {
    backgroundColor: "var(--find-match)",
    borderRadius: "2px",
  },
  ".cm-findCurrent": {
    backgroundColor: "var(--find-current)",
    boxShadow: "0 0 0 1px var(--find-current-ring)",
  },
});

/**
 * Mod+F (and the user's binding for `find`) opens the find bar, before any
 * of the editor's own keymaps: CodeMirror's search panel never opens.
 */
export const findKeys: Extension = Prec.highest(
  EditorView.domEventHandlers({
    keydown(e) {
      const ours = findAction(e, currentKeymap(), "app") === "find";
      if (!ours && !matchesBinding(e, "Mod+KeyF")) return false;
      e.preventDefault();
      if (ours) openFindFrom(e.target);
      return true;
    },
  }),
);

/** The highlighting, for an editor with a `findTarget`. */
export const findHighlighting: Extension = [findState, findMarks, findTheme];
