/**
 * The CodeMirror 6 setup for the code view, ported from Selene's `SqlEditor`:
 * the GitHub light/dark themes (`@uiw/codemirror-theme-github`) for syntax
 * colours, with Selene's font and gutter styling on top at `Prec.highest` —
 * bound to the design tokens instead of Selene's retro hex overrides, so dark,
 * light and retro all follow `tokens.css`. Read-only: no keymap, no
 * autocompletion, no active line.
 *
 * Languages come from `@codemirror/language-data`, matched by file name (the
 * diffs) or a code fence's info string (comment code blocks) and loaded
 * lazily (each is its own chunk). jq, common in review comments but not in
 * language-data, has a small stream mode here.
 *
 * Comment code blocks (`codeBlockExtensions`) use the same themes, without
 * line numbers and a size smaller.
 *
 * Every editor takes Mod+F to the find bar (`lib/findHighlight.findKeys`),
 * never to CodeMirror's own search.
 */

import {
  LanguageDescription,
  LanguageSupport,
  StreamLanguage,
} from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { EditorState, Prec, type Extension } from "@codemirror/state";
import {
  drawSelection,
  EditorView,
  highlightSpecialChars,
  lineNumbers,
} from "@codemirror/view";
import {
  githubDark,
  githubDarkInit,
  githubDarkStyle,
  githubLight,
  githubLightInit,
  githubLightStyle,
} from "@uiw/codemirror-theme-github";

import { isDarkTheme, type ThemeMode } from "../state/themeStore";
import { findKeys } from "./findHighlight";

/** Selene's editor look, on the Argus tokens (every theme). */
const tokenTheme = Prec.highest(
  EditorView.theme({
    "&": {
      background: "var(--bg)",
      color: "var(--text)",
      fontSize: "var(--fs-md)",
    },
    "&.cm-focused": { outline: "none" },
    ".cm-scroller": {
      fontFamily: "var(--font-mono)",
      lineHeight: "1.55",
    },
    ".cm-content": { paddingBlock: "4px" },
    ".cm-gutters": {
      background: "var(--surface)",
      borderRight: "1px solid var(--border)",
      color: "var(--text-faint)",
    },
    ".cm-lineNumbers .cm-gutterElement": {
      padding: "0 6px 0 10px",
      minWidth: "32px",
    },
    ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
      background: "var(--selection) !important",
    },
    // Diff colours: the merge package's classes on the semantic tokens
    // (its base theme hard-codes light/dark colours). `cm-merge-a` is the
    // left editor of the split view; the unified editor is `cm-merge-b`.
    "&.cm-merge-a .cm-changedLine, .cm-deletedChunk": {
      backgroundColor: "var(--diff-del-bg)",
    },
    "&.cm-merge-b .cm-changedLine, .cm-inlineChangedLine": {
      backgroundColor: "var(--diff-add-bg)",
    },
    "&.cm-merge-a .cm-changedText, .cm-deletedChunk .cm-deletedText, &.cm-merge-b .cm-deletedText":
      { background: "var(--diff-del-strong)" },
    "&.cm-merge-b .cm-changedText": { background: "var(--diff-add-strong)" },
    ".cm-insertedLine, .cm-deletedLine, .cm-deletedLine del": {
      textDecoration: "none",
    },
    ".cm-changeGutter": { width: "3px", paddingLeft: "0" },
    "&.cm-merge-a .cm-changedLineGutter, .cm-deletedLineGutter": {
      background: "var(--danger)",
    },
    "&.cm-merge-b .cm-changedLineGutter": { background: "var(--success)" },
    ".cm-collapsedLines": {
      background: "var(--surface-2)",
      color: "var(--text-muted)",
      fontFamily: "var(--font-sans)",
      fontSize: "var(--fs-sm)",
      padding: "3px 12px",
      cursor: "pointer",
    },
    ".cm-collapsedLines:hover": {
      background: "var(--surface-hover)",
      color: "var(--text)",
    },
    ".cm-collapsedLines:before, .cm-collapsedLines:after": { content: "none" },
  }),
);

/**
 * Glass themes: the editor and gutters go transparent so the code sits on
 * the glass card, whose fill is what the syntax colours need 4.5:1 on. The
 * GitHub colours miss that on the pastel canvas (light) and the teal patch
 * (dark comments), so those get darker / lighter twins.
 */
const glassClear = Prec.highest(
  EditorView.theme({
    "&": { background: "transparent" },
    ".cm-gutters": { background: "transparent" },
  }),
);
/** The GitHub styles with some colours swapped (`from` hex -> `to`). */
function recolor(
  base: typeof githubLightStyle,
  swap: Record<string, string>,
): typeof githubLightStyle {
  return base.map((r) => ({ ...r, color: swap[r.color ?? ""] ?? r.color }));
}
const glassLight = githubLightInit({
  styles: recolor(githubLightStyle, {
    "#6a737d": "#57606a", // comment, bracket
    "#d73a49": "#b31d28", // keyword, type
    "#cb2431": "#b31d28", // invalid
    "#22863a": "#116329", // name, quote
    "#e36209": "#a14100", // atom, bool
  }),
});
const glassDark = githubDarkInit({
  styles: recolor(githubDarkStyle, { "#8b949e": "#9aa4af" }), // comment
});

/** The CodeMirror theme for `mode`: GitHub dark/light, tuned on glass. */
function themeFor(mode: ThemeMode): Extension {
  if (mode === "glass-dark") return [glassDark, glassClear];
  if (mode === "glass-light") return [glassLight, glassClear];
  return isDarkTheme(mode) ? githubDark : githubLight;
}

/** The read-only base every diff editor gets. */
export function baseExtensions(mode: ThemeMode): Extension[] {
  return [
    themeFor(mode),
    tokenTheme,
    findKeys,
    lineNumbers(),
    highlightSpecialChars(),
    drawSelection(),
    EditorState.readOnly.of(true),
    EditorView.editable.of(false),
  ];
}

/** A comment's fenced code block: compact, no gutter, read-only. */
const codeBlockTheme = Prec.highest(
  EditorView.theme({
    "&": { fontSize: "var(--fs-sm)", background: "transparent" },
    ".cm-content": { padding: "6px 0" },
    ".cm-line": { padding: "0 10px" },
  }),
);

/** The read-only base of a comment's code block. */
export function codeBlockExtensions(mode: ThemeMode): Extension[] {
  return [
    themeFor(mode),
    tokenTheme,
    codeBlockTheme,
    findKeys,
    highlightSpecialChars(),
    drawSelection(),
    EditorState.readOnly.of(true),
    EditorView.editable.of(false),
  ];
}

const loaded = new Map<string, Promise<LanguageSupport | null>>();

function load(desc: LanguageDescription): Promise<LanguageSupport | null> {
  let p = loaded.get(desc.name);
  if (!p) {
    p = desc.load().catch(() => null);
    loaded.set(desc.name, p);
  }
  return p;
}

/** The language for `path` by file name, loaded once; `null` when unknown. */
export function languageFor(path: string): Promise<LanguageSupport | null> {
  const name = path.split("/").pop() ?? path;
  const desc = LanguageDescription.matchFilename(languages, name);
  return desc ? load(desc) : Promise.resolve(null);
}

/**
 * The language for a code fence's info string (` ```ts `, ` ```rust ` …):
 * its first word as a language name or alias, else as a file extension
 * (` ```rs `, ` ```py `); `null` when unknown (shown as plain text).
 */
export function languageForInfo(info: string): Promise<LanguageSupport | null> {
  const name = info.trim().split(/\s+/)[0]?.toLowerCase() ?? "";
  if (!name) return Promise.resolve(null);
  if (name === "jq") return Promise.resolve(jq());
  const desc =
    LanguageDescription.matchLanguageName(languages, name, false) ??
    LanguageDescription.matchFilename(languages, `file.${name}`);
  return desc ? load(desc) : Promise.resolve(null);
}

// -- jq -------------------------------------------------------------------

const JQ_KEYWORDS = new Set([
  "def",
  "if",
  "then",
  "elif",
  "else",
  "end",
  "as",
  "reduce",
  "foreach",
  "try",
  "catch",
  "label",
  "import",
  "include",
  "and",
  "or",
  "not",
]);
const JQ_ATOMS = new Set(["true", "false", "null"]);

/** Inside a string literal across tokens (`\(…)` stays part of it). */
interface JqState {
  inString: boolean;
}

const jqLanguage = StreamLanguage.define<JqState>({
  name: "jq",
  startState: () => ({ inString: false }),
  token(stream, state) {
    if (state.inString) {
      let escaped = false;
      for (let ch = stream.next(); ch != null; ch = stream.next()) {
        if (ch === '"' && !escaped) {
          state.inString = false;
          break;
        }
        escaped = !escaped && ch === "\\";
      }
      return "string";
    }
    if (stream.eatSpace()) return null;
    if (stream.peek() === "#") {
      stream.skipToEnd();
      return "comment";
    }
    if (stream.eat('"')) {
      state.inString = true;
      return "string";
    }
    if (stream.match(/^\d+(\.\d+)?([eE][+-]?\d+)?/)) return "number";
    if (stream.match(/^\$[A-Za-z_]\w*/)) return "variableName";
    if (stream.match(/^\.[A-Za-z_]\w*/)) return "propertyName";
    if (stream.match(/^@[A-Za-z]\w*/)) return "string.special";
    if (stream.match(/^[A-Za-z_]\w*(::[A-Za-z_]\w*)*/)) {
      const word = stream.current();
      if (JQ_KEYWORDS.has(word)) return "keyword";
      if (JQ_ATOMS.has(word)) return "atom";
      return "variableName.function";
    }
    if (
      stream.match(/^(\?\/\/|\/\/=?|[|+\-*/%]=|==|!=|<=|>=|[|,+\-*/%<>=?])/)
    ) {
      return "operator";
    }
    stream.next();
    return null;
  },
  languageData: { commentTokens: { line: "#" } },
});

let jqSupport: LanguageSupport | null = null;
function jq(): LanguageSupport {
  jqSupport ??= new LanguageSupport(jqLanguage);
  return jqSupport;
}
