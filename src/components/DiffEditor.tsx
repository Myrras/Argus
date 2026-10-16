/**
 * One file's diff in CodeMirror 6 (`@codemirror/merge`): unified
 * (`unifiedMergeView`, the head with the removed lines as blocks) or side by
 * side (`MergeView`, base left, head right). Read-only, syntax-highlighted
 * by file name, unchanged stretches collapsed (click to expand).
 *
 * Threads sit in block widgets under their line, each its own React root
 * (`renderThread`). A collapsed stretch holding a thread is expanded up
 * front so no thread hides. The gutter's "+" is the hook for commenting
 * (milestone 4): shown on hover, disabled, with a tooltip.
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
import type { DiffLayout } from "../state/layoutStore";
import { useThemeStore } from "../state/themeStore";

const COLLAPSE = { margin: 3, minSize: 4 };

export type RenderThread = (
  thread: StoredThread,
  onResize: () => void,
) => ReactNode;

/** Threads under one line. */
class ThreadsWidget extends WidgetType {
  private root: Root | null = null;

  constructor(
    readonly threads: readonly StoredThread[],
    readonly render: RenderThread,
  ) {
    super();
  }

  /** Same threads in the same state: keep the DOM (and its open/closed). */
  eq(other: ThreadsWidget): boolean {
    return key(other.threads) === key(this.threads);
  }

  toDOM(view: EditorView): HTMLElement {
    const dom = document.createElement("div");
    dom.className = "cm-argusThreads";
    // The content scrolls sideways under long lines; the threads stay put
    // beside the gutter, as wide as the visible part.
    const gutters = view.dom.querySelector<HTMLElement>(".cm-gutters");
    const left = gutters?.offsetWidth ?? 0;
    dom.style.left = `${left}px`;
    dom.style.width = `${Math.max(240, view.scrollDOM.clientWidth - left - 8)}px`;
    this.root = createRoot(dom);
    const resize = () => view.requestMeasure();
    this.root.render(
      <>
        {this.threads.map((t) => (
          <div key={t.threadId}>{this.render(t, resize)}</div>
        ))}
      </>,
    );
    return dom;
  }

  destroy(): void {
    const root = this.root;
    this.root = null;
    // Not during CodeMirror's own update (React warns on a sync unmount
    // inside another render).
    if (root) queueMicrotask(() => root.unmount());
  }

  /** Clicks and keys belong to the thread's buttons, not the editor. */
  ignoreEvent(): boolean {
    return true;
  }
}

function key(threads: readonly StoredThread[]): string {
  return threads
    .map((t) => `${t.threadId}:${t.notes.length}:${t.resolved}`)
    .join("|");
}

/** Lines (1-based) with their threads, for one editor. */
type LineThreads = Map<number, StoredThread[]>;

const setThreads = StateEffect.define<{
  lines: LineThreads;
  render: RenderThread;
}>();

const threadField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    for (const e of tr.effects) {
      if (!e.is(setThreads)) continue;
      const builder = new RangeSetBuilder<Decoration>();
      const doc = tr.state.doc;
      for (const line of [...e.value.lines.keys()].sort((x, y) => x - y)) {
        if (line > doc.lines) continue;
        builder.add(
          doc.line(line).to,
          doc.line(line).to,
          Decoration.widget({
            widget: new ThreadsWidget(
              e.value.lines.get(line) ?? [],
              e.value.render,
            ),
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

/** The disabled "+" in the gutter: where commenting will start (M4). */
class CommentMarker extends GutterMarker {
  toDOM(): Node {
    const el = document.createElement("span");
    el.className = "cm-argusCommentPlus";
    el.textContent = "+";
    el.title = "Commenting comes in a later release";
    el.setAttribute("aria-disabled", "true");
    return el;
  }
}

const commentMarker = new CommentMarker();

const commentGutter: Extension = [
  gutter({
    class: "cm-argusCommentGutter",
    lineMarker: () => commentMarker,
    initialSpacer: () => commentMarker,
  }),
  EditorView.theme({
    ".cm-argusCommentGutter .cm-gutterElement": {
      width: "16px",
      textAlign: "center",
      color: "var(--text-faint)",
      cursor: "not-allowed",
    },
    ".cm-argusCommentPlus": { opacity: "0" },
    ".cm-argusCommentGutter .cm-gutterElement:hover .cm-argusCommentPlus": {
      opacity: "1",
    },
    ".cm-argusThreads": { padding: "2px 0", position: "sticky" },
  }),
];

function group(placements: readonly Placement[], side: DiffSide): LineThreads {
  const out: LineThreads = new Map();
  for (const p of placements) {
    if (p.side !== side) continue;
    out.set(p.line, [...(out.get(p.line) ?? []), p.thread]);
  }
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
}) {
  const host = useRef<HTMLDivElement>(null);
  const mode = useThemeStore((s) => s.mode);
  const views = useRef<EditorView[]>([]);
  const theme = useRef(new Compartment());
  const language = useRef(new Compartment());
  // The latest props for the (re)build effect without rebuilding on each.
  const latest = useRef({ placements, renderThread, mode });
  latest.current = { placements, renderThread, mode };

  // Build the editor(s) for this content and layout.
  useEffect(() => {
    const parent = host.current;
    if (!parent) return;
    const { mode: m } = latest.current;
    const common = (): Extension[] => [
      theme.current.of(baseExtensions(m)),
      language.current.of([]),
      threadField,
      commentGutter,
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

    let cancelled = false;
    void languageFor(path).then((lang) => {
      if (cancelled || !lang) return;
      for (const v of views.current) {
        v.dispatch({ effects: language.current.reconfigure(lang) });
      }
    });

    return () => {
      cancelled = true;
      merge?.destroy();
      unified?.destroy();
      views.current = [];
    };
  }, [path, original, modified, layout]);

  // Place the threads (again whenever they change).
  useEffect(() => {
    const vs = views.current;
    if (vs.length === 0) return;
    const { renderThread: render } = latest.current;
    if (vs.length === 2) {
      const [a, b] = vs;
      const onA = group(placements, "a");
      const onB = group(placements, "b");
      uncollapseAt(a, "a", [...onA.keys()]);
      uncollapseAt(b, "b", [...onB.keys()]);
      a.dispatch({ effects: setThreads.of({ lines: onA, render }) });
      b.dispatch({ effects: setThreads.of({ lines: onB, render }) });
    } else {
      // Unified: base-side threads go where those lines show on the head.
      const [view] = vs;
      const info = getChunks(view.state);
      const lines: LineThreads = group(placements, "b");
      if (info) {
        const origDoc = EditorState.create({ doc: original }).doc;
        for (const p of placements) {
          if (p.side !== "a" || p.line > origDoc.lines) continue;
          const at = mapOldLine(info.chunks, origDoc, view.state.doc, p.line);
          lines.set(at, [...(lines.get(at) ?? []), p.thread]);
        }
      }
      uncollapseAt(view, "b", [...lines.keys()]);
      view.dispatch({ effects: setThreads.of({ lines, render }) });
    }
  }, [placements, original, modified, layout]);

  // Theme switches reconfigure in place.
  useEffect(() => {
    for (const v of views.current) {
      v.dispatch({ effects: theme.current.reconfigure(baseExtensions(mode)) });
    }
  }, [mode]);

  return <div ref={host} className="argus-diff-editor" data-layout={layout} />;
}
