/**
 * A comment's fenced code block in CodeMirror (`lib/codemirror`'s
 * `codeBlockExtensions`): read-only, highlighted by the fence's info string,
 * following the theme. Loaded lazily by `Markdown`'s `CodeBlock`, which shows
 * the plain text until it is in.
 */

import { useEffect, useRef } from "react";

import { Compartment, EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

import { codeBlockExtensions, languageForInfo } from "../lib/codemirror";
import { useThemeStore } from "../state/themeStore";

export function CodeEditorBlock({
  code,
  info,
}: {
  code: string;
  info: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const theme = useRef(new Compartment());
  const language = useRef(new Compartment());
  const mode = useThemeStore((s) => s.mode);
  const modeRef = useRef(mode);
  modeRef.current = mode;

  useEffect(() => {
    const parent = host.current;
    if (!parent) return;
    const v = new EditorView({
      parent,
      state: EditorState.create({
        doc: code,
        extensions: [
          theme.current.of(codeBlockExtensions(modeRef.current)),
          language.current.of([]),
        ],
      }),
    });
    view.current = v;
    let cancelled = false;
    void languageForInfo(info).then((lang) => {
      if (!cancelled && lang) {
        v.dispatch({ effects: language.current.reconfigure(lang) });
      }
    });
    return () => {
      cancelled = true;
      v.destroy();
      view.current = null;
    };
  }, [code, info]);

  useEffect(() => {
    view.current?.dispatch({
      effects: theme.current.reconfigure(codeBlockExtensions(mode)),
    });
  }, [mode]);

  return <div ref={host} className="argus-code-block" data-language={info} />;
}
