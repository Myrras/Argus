/**
 * One column of a thread's then/now code in CodeMirror (`lib/codemirror`'s
 * `codeBlockExtensions` plus a gutter): read-only, highlighted by file name,
 * following the theme. The line numbers are the file's, blank on pad lines;
 * changed lines carry the diff colours and the commented lines an accent
 * edge, as line classes with `data-kind` / `data-anchored`.
 *
 * Loaded lazily by `ThreadHistory`, which shows the same lines as plain
 * monospaced rows until it is in.
 */

import { useEffect, useRef } from "react";

import {
  Compartment,
  EditorState,
  Prec,
  RangeSetBuilder,
} from "@codemirror/state";
import { Decoration, EditorView, lineNumbers } from "@codemirror/view";

import { codeBlockExtensions, languageFor } from "../lib/codemirror";
import type { ColumnLine } from "../lib/threadHistory";
import { useThemeStore } from "../state/themeStore";

const columnTheme = Prec.highest(
  EditorView.theme({
    ".cm-gutters": { background: "transparent" },
    ".cm-lineNumbers .cm-gutterElement": {
      padding: "0 6px 0 8px",
      minWidth: "28px",
    },
    ".cm-argusTnDel": { backgroundColor: "var(--diff-del-bg)" },
    ".cm-argusTnAdd": { backgroundColor: "var(--diff-add-bg)" },
    ".cm-argusTnPad": { backgroundColor: "var(--surface-2)" },
    ".cm-argusTnAnchored": { boxShadow: "inset 3px 0 0 var(--accent)" },
  }),
);

const KIND_CLASS: Record<ColumnLine["kind"], string> = {
  same: "",
  del: "cm-argusTnDel",
  add: "cm-argusTnAdd",
  pad: "cm-argusTnPad",
};

/** A line decoration per line; static, as the doc never changes. */
function decorations(lines: readonly ColumnLine[]) {
  const b = new RangeSetBuilder<Decoration>();
  let at = 0;
  for (const l of lines) {
    const cls = [KIND_CLASS[l.kind], l.anchored ? "cm-argusTnAnchored" : ""]
      .filter(Boolean)
      .join(" ");
    b.add(
      at,
      at,
      Decoration.line({
        class: cls,
        attributes: {
          "data-kind": l.kind,
          "data-anchored": String(l.anchored),
        },
      }),
    );
    at += l.text.length + 1;
  }
  return b.finish();
}

export function ThenNowEditor({
  lines,
  path,
  onReady,
}: {
  lines: readonly ColumnLine[];
  path: string;
  /** The editor replaced the plain rows (its height may differ). */
  onReady?: () => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const theme = useRef(new Compartment());
  const language = useRef(new Compartment());
  const mode = useThemeStore((s) => s.mode);
  const modeRef = useRef(mode);
  modeRef.current = mode;
  const ready = useRef(onReady);
  ready.current = onReady;

  useEffect(() => {
    const parent = host.current;
    if (!parent) return;
    const widest = String(Math.max(0, ...lines.map((l) => l.line ?? 0)));
    const v = new EditorView({
      parent,
      state: EditorState.create({
        doc: lines.map((l) => l.text).join("\n"),
        extensions: [
          theme.current.of(codeBlockExtensions(modeRef.current)),
          // Past the last line: CodeMirror sizes the gutter by a number as
          // wide as the doc's line count; the file's widest number instead.
          lineNumbers({
            formatNumber: (n) =>
              n > lines.length ? widest : String(lines[n - 1].line ?? ""),
          }),
          EditorView.decorations.of(decorations(lines)),
          columnTheme,
          language.current.of([]),
        ],
      }),
    });
    view.current = v;
    ready.current?.();
    let cancelled = false;
    void languageFor(path).then((lang) => {
      if (!cancelled && lang) {
        v.dispatch({ effects: language.current.reconfigure(lang) });
      }
    });
    return () => {
      cancelled = true;
      v.destroy();
      view.current = null;
    };
  }, [lines, path]);

  useEffect(() => {
    view.current?.dispatch({
      effects: theme.current.reconfigure(codeBlockExtensions(mode)),
    });
  }, [mode]);

  return <div ref={host} className="argus-then-now" data-path={path} />;
}
