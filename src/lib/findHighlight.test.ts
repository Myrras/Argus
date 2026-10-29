import { MergeView, unifiedMergeView } from "@codemirror/merge";
import { EditorState, Prec, type Extension } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./platform", () => ({ isMac: true, isWindows: false }));

import {
  publishFindHighlight,
  setFindOpener,
  type FindHighlightState,
} from "./findBridge";
import {
  findDecorations,
  findHighlighting,
  findKeys,
  findState,
  findTarget,
  setFindState,
  type FindTarget,
} from "./findHighlight";
import { codeUnit } from "./findInReview";
import { compileFind } from "./findQuery";

const re = (text: string) =>
  compileFind(text, { caseSensitive: false, wholeWord: false, regex: false })
    .re!;

function find(over: Partial<FindHighlightState> = {}): FindHighlightState {
  return { re: re("foo"), scope: "everything", current: null, ...over };
}

/** The decorations of `state` over its whole document. */
function marks(state: EditorState) {
  const out: { text: string; cls: string }[] = [];
  const set = findDecorations(state, [{ from: 0, to: state.doc.length }]);
  set.between(0, state.doc.length, (from, to, deco) => {
    out.push({
      text: state.sliceDoc(from, to),
      cls: String(deco.spec.class),
    });
  });
  return out;
}

function withFind(
  doc: string,
  target: FindTarget,
  value: FindHighlightState,
  extra: Extension[] = [],
): EditorState {
  const state = EditorState.create({
    doc,
    extensions: [findHighlighting, findTarget.of(target), ...extra],
  });
  return state.update({ effects: setFindState.of(value) }).state;
}

const head: FindTarget = {
  kind: "diff",
  path: "src/a.rs",
  side: "new",
  unified: true,
};

afterEach(() => {
  publishFindHighlight({ re: null, scope: "everything", current: null });
  setFindOpener(null);
});

describe("findDecorations", () => {
  it("marks every match of a head side, the current one standing out", () => {
    const state = withFind(
      "foo bar\nbar foo foo",
      head,
      find({
        current: {
          unit: codeUnit("src/a.rs", "new"),
          line: 2,
          start: 8,
          end: 11,
        },
      }),
      [unifiedMergeView({ original: "" })],
    );
    expect(marks(state)).toEqual([
      { text: "foo", cls: "cm-findMatch" },
      { text: "foo", cls: "cm-findMatch" },
      { text: "foo", cls: "cm-findMatch cm-findCurrent" },
    ]);
  });

  it("follows the scope and shows nothing without a query or target", () => {
    expect(marks(withFind("foo", head, find({ scope: "comments" })))).toEqual(
      [],
    );
    expect(marks(withFind("foo", head, find({ re: null })))).toEqual([]);
    const bare = EditorState.create({
      doc: "foo",
      extensions: [findHighlighting],
    });
    expect(marks(bare)).toEqual([]);
  });

  it("on a base side, marks only the removed lines (unchanged ones count on the head)", () => {
    const parent = document.createElement("div");
    document.body.appendChild(parent);
    const target = (side: "old" | "new"): Extension[] => [
      findHighlighting,
      findTarget.of({ kind: "diff", path: "src/a.rs", side, unified: false }),
    ];
    const view = new MergeView({
      a: { doc: "foo kept\nfoo gone\nfoo kept", extensions: target("old") },
      b: { doc: "foo kept\nfoo new\nfoo kept", extensions: target("new") },
      parent,
    });
    const a = view.a.state.update({ effects: setFindState.of(find()) }).state;
    const b = view.b.state.update({ effects: setFindState.of(find()) }).state;
    expect(marks(a).map((m) => m.text)).toEqual(["foo"]);
    const first = findDecorations(a, [{ from: 0, to: a.doc.length }]).iter();
    expect(a.doc.lineAt(first.from).number).toBe(2);
    expect(marks(b)).toHaveLength(3);
    view.destroy();
    parent.remove();
  });

  it("counts a comment's code block from where it starts in the comment", () => {
    const target: FindTarget = { kind: "text", unit: "note:t:n", base: 20 };
    const state = withFind(
      "x foo\nfoo",
      target,
      // The comment's text has the code at offset 20: "foo" on line 2 is 26.
      find({ current: { unit: "note:t:n", start: 26, end: 29 } }),
    );
    expect(marks(state)).toEqual([
      { text: "foo", cls: "cm-findMatch" },
      { text: "foo", cls: "cm-findMatch cm-findCurrent" },
    ]);
    expect(marks(withFind("foo", target, find({ scope: "code" })))).toEqual([]);
  });

  it("starts from the latest state and follows the bridge", async () => {
    publishFindHighlight(find());
    const parent = document.createElement("div");
    const view = new EditorView({
      parent,
      state: EditorState.create({
        doc: "foo",
        extensions: [findHighlighting, findTarget.of(head)],
      }),
    });
    expect(view.state.field(findState).re?.source).toBe("foo");
    publishFindHighlight(find({ re: re("bar") }));
    expect(view.state.field(findState).re?.source).toBe("bar");
    view.destroy();
  });
});

describe("findKeys", () => {
  it("takes Mod+F to the find bar, never to CodeMirror's own search", () => {
    const opened = vi.fn(() => true);
    setFindOpener(opened);
    const cmSearch = vi.fn(() => true);
    const parent = document.createElement("div");
    document.body.appendChild(parent);
    const view = new EditorView({
      parent,
      state: EditorState.create({
        doc: "foo",
        // A search keymap (as @codemirror/search binds it) under ours.
        extensions: [
          findKeys,
          Prec.high(keymap.of([{ key: "Mod-f", run: cmSearch }])),
        ],
      }),
    });
    const e = new KeyboardEvent("keydown", {
      key: "f",
      code: "KeyF",
      metaKey: true,
      bubbles: true,
      cancelable: true,
    });
    view.contentDOM.dispatchEvent(e);
    expect(opened).toHaveBeenCalledWith(view.contentDOM);
    expect(cmSearch).not.toHaveBeenCalled();
    expect(e.defaultPrevented).toBe(true);
    // Other keys pass.
    const g = new KeyboardEvent("keydown", {
      key: "g",
      code: "KeyG",
      metaKey: true,
      bubbles: true,
      cancelable: true,
    });
    view.contentDOM.dispatchEvent(g);
    expect(g.defaultPrevented).toBe(false);
    view.destroy();
    parent.remove();
  });
});
