import { describe, expect, it } from "vitest";

import {
  ACTIONS,
  DEFAULT_KEYMAP,
  activeActions,
  bindingFromEvent,
  bindingProblem,
  findAction,
  findConflict,
  formatBinding,
  isBrowserShortcut,
  matches,
  ownsEnter,
  parseBinding,
  resolveKeymap,
  serializeBinding,
  sharesByContext,
  shortcutHint,
  withHint,
  type ActionId,
  type KeyEventLike,
} from "./keymap";

function ev(init: Partial<KeyEventLike> & { key: string }): KeyEventLike {
  return {
    code: "",
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    ...init,
  };
}

describe("matches", () => {
  it("uses Cmd on macOS and Ctrl elsewhere", () => {
    const cmd = ev({ key: "p", metaKey: true });
    const ctrl = ev({ key: "p", ctrlKey: true });
    expect(matches(cmd, "Mod+KeyP", true)).toBe(true);
    expect(matches(ctrl, "Mod+KeyP", true)).toBe(false);
    expect(matches(ctrl, "Mod+KeyP", false)).toBe(true);
    expect(matches(cmd, "Mod+KeyP", false)).toBe(false);
  });

  it("is exact about Shift and Alt", () => {
    expect(
      matches(
        ev({ key: "r", ctrlKey: true, shiftKey: true }),
        "Mod+KeyR",
        false,
      ),
    ).toBe(false);
    expect(
      matches(
        ev({ key: "®", code: "KeyR", ctrlKey: true, altKey: true }),
        "Mod+KeyR",
        false,
      ),
    ).toBe(false);
    expect(matches(ev({ key: "r" }), "Mod+KeyR", false)).toBe(false);
  });

  it("matches a letter by its character, whatever the case or layout", () => {
    // Shift gives "N"; a Dvorak user's physical key is another one.
    expect(
      matches(
        ev({ key: "N", code: "KeyL", ctrlKey: true, shiftKey: true }),
        "Mod+Shift+KeyN",
        false,
      ),
    ).toBe(true);
  });

  it("matches Option combinations by the physical key", () => {
    // Option+S types "ß", Option+R "®", Option+P "π" on a Mac.
    const sync = ev({ key: "ß", code: "KeyS", metaKey: true, altKey: true });
    expect(matches(sync, "Mod+Alt+KeyS", true)).toBe(true);
    expect(matches(sync, "Mod+KeyS", true)).toBe(false);
    expect(
      matches(
        ev({ key: "π", code: "KeyP", metaKey: true, altKey: true }),
        "Mod+Alt+KeyP",
        true,
      ),
    ).toBe(true);
  });

  it("falls back to the key when the code is missing or Unknown", () => {
    expect(
      matches(
        ev({ key: ",", code: "Unknown", ctrlKey: true }),
        "Mod+Comma",
        false,
      ),
    ).toBe(true);
    expect(matches(ev({ key: "F2" }), "F2", false)).toBe(true);
    expect(matches(ev({ key: "Delete" }), "Delete", false)).toBe(true);
    expect(
      matches(ev({ key: "Backspace", ctrlKey: true }), "Mod+Backspace", false),
    ).toBe(true);
    expect(
      matches(
        ev({ key: "]", code: "BracketRight", ctrlKey: true, shiftKey: true }),
        "Mod+Shift+BracketRight",
        false,
      ),
    ).toBe(true);
  });

  it("ignores Ctrl on a Mac and the Windows key elsewhere", () => {
    const both = ev({ key: "p", metaKey: true, ctrlKey: true });
    expect(matches(both, "Mod+KeyP", true)).toBe(false);
    expect(matches(both, "Mod+KeyP", false)).toBe(false);
  });

  it("never matches a bare modifier press", () => {
    expect(matches(ev({ key: "Meta", metaKey: true }), "Mod+KeyP", true)).toBe(
      false,
    );
  });
});

describe("formatBinding", () => {
  it("shows Shift+/ as ?, and matches ? on any layout", () => {
    expect(formatBinding("Shift+Slash", true)).toBe("?");
    expect(formatBinding("Shift+Slash", false)).toBe("?");
    expect(formatBinding("Mod+Shift+Slash", false)).toBe("Ctrl+Shift+/");
    expect(matches(ev({ key: "?", shiftKey: true }), "Shift+Slash")).toBe(true);
    // German: Shift+ß types "?".
    expect(
      matches(ev({ key: "?", code: "Minus", shiftKey: true }), "Shift+Slash"),
    ).toBe(true);
  });

  it("draws macOS glyphs: Option, Shift, Command, then the key", () => {
    expect(formatBinding("Mod+Shift+KeyG", true)).toBe("⇧⌘G");
    expect(formatBinding("Mod+Alt+KeyS", true)).toBe("⌥⌘S");
    expect(formatBinding("Mod+Shift+KeyN", true)).toBe("⇧⌘N");
    expect(formatBinding("Mod+Backspace", true)).toBe("⌘⌫");
    expect(formatBinding("Mod+Comma", true)).toBe("⌘,");
    expect(formatBinding("Mod+Shift+BracketLeft", true)).toBe("⇧⌘[");
    expect(formatBinding("F2", true)).toBe("F2");
    expect(formatBinding("Delete", true)).toBe("⌦");
  });

  it("spells it out elsewhere: Ctrl+Alt+Shift+key", () => {
    expect(formatBinding("Mod+Shift+KeyG", false)).toBe("Ctrl+Shift+G");
    expect(formatBinding("Mod+Alt+KeyS", false)).toBe("Ctrl+Alt+S");
    expect(formatBinding("Mod+Digit1", false)).toBe("Ctrl+1");
    expect(formatBinding("Mod+Backspace", false)).toBe("Ctrl+Backspace");
    expect(formatBinding("Delete", false)).toBe("Delete");
    expect(formatBinding("Mod+Alt+Shift+ArrowUp", false)).toBe(
      "Ctrl+Alt+Shift+↑",
    );
  });

  it("returns a hint for the first binding, or nothing when unbound", () => {
    expect(shortcutHint(DEFAULT_KEYMAP, "refresh", true)).toBe("⇧R");
    expect(withHint("Settings", DEFAULT_KEYMAP, "openSettings", false)).toBe(
      "Settings (Ctrl+,)",
    );
    const unbound = { ...DEFAULT_KEYMAP, refresh: [] };
    expect(withHint("Refresh", unbound, "refresh", false)).toBe("Refresh");
  });
});

describe("parse and serialize", () => {
  it("round-trips and rejects nonsense", () => {
    for (const text of ["Mod+Shift+KeyN", "Mod+Alt+KeyS", "F2", "Delete"]) {
      expect(serializeBinding(parseBinding(text)!)).toBe(text);
    }
    expect(parseBinding("")).toBeNull();
    expect(parseBinding("Cmd+KeyP")).toBeNull();
    expect(parseBinding("Mod+")).toBeNull();
  });

  it("writes the modifiers in a fixed order", () => {
    expect(
      serializeBinding({ code: "KeyG", mod: true, shift: true, alt: true }),
    ).toBe("Mod+Alt+Shift+KeyG");
  });
});

describe("defaults", () => {
  it("are the list keys Argus always had; refresh is ⇧R and Cmd/Ctrl+R", () => {
    expect(DEFAULT_KEYMAP.nextReview).toEqual(["ArrowDown"]);
    expect(DEFAULT_KEYMAP.prevReview).toEqual(["ArrowUp"]);
    expect(DEFAULT_KEYMAP.openReview).toEqual(["Enter"]);
    // Bare R replies to the focused thread since M6.
    expect(DEFAULT_KEYMAP.refresh).toEqual(["Shift+KeyR", "Mod+KeyR"]);
    expect(DEFAULT_KEYMAP.replyThread).toEqual(["KeyR"]);
    expect(DEFAULT_KEYMAP.showNeedsReview).toEqual(["Digit1"]);
    expect(DEFAULT_KEYMAP.showClosedWithoutApproval).toEqual(["Digit4"]);
    expect(DEFAULT_KEYMAP.openSettings).toEqual(["Mod+Comma"]);
  });

  it("all parse, none repeats, none is fixed or refused", () => {
    for (const mac of [true, false]) {
      const seen = new Map<string, string>();
      for (const a of activeActions(mac)) {
        for (const b of a.defaults) {
          const parsed = parseBinding(b);
          expect(parsed, `${a.id} ${b}`).not.toBeNull();
          // A fixed action (Escape) is listed, never recorded.
          if (!a.fixed) {
            expect(bindingProblem(parsed!, mac), `${a.id} ${b}`).toBeNull();
          }
          const other = seen.get(b);
          // ↑/↓: the list's and the review page's, by where the focus is.
          if (!other || !sharesByContext(a.id, other as ActionId)) {
            expect(other, `${b} used by ${a.id} and ${other}`).toBe(undefined);
          }
          seen.set(b, a.id);
        }
      }
    }
  });

  it("have a known action for every id exactly once", () => {
    expect(new Set(ACTIONS.map((a) => a.id)).size).toBe(ACTIONS.length);
    expect(Object.keys(DEFAULT_KEYMAP).sort()).toEqual(
      ACTIONS.map((a) => a.id).sort(),
    );
  });

  it("leave Settings to the native menu on macOS", () => {
    expect(activeActions(true).map((a) => a.id)).not.toContain("openSettings");
    expect(activeActions(false).map((a) => a.id)).toContain("openSettings");
  });
});

describe("comment keys", () => {
  it("are c, Mod+Enter and a fixed Escape", () => {
    expect(DEFAULT_KEYMAP.commentLine).toEqual(["KeyC"]);
    expect(DEFAULT_KEYMAP.submitComment).toEqual(["Mod+Enter"]);
    expect(DEFAULT_KEYMAP.cancelComment).toEqual(["Escape"]);
    // A stored override cannot move the fixed one.
    expect(resolveKeymap({ cancelComment: ["KeyQ"] }).cancelComment).toEqual([
      "Escape",
    ]);
  });

  it("keep the composer keys out of the window-wide scopes", () => {
    const enter = {
      key: "Enter",
      code: "Enter",
      metaKey: true,
      ctrlKey: false,
      altKey: false,
      shiftKey: false,
    };
    expect(findAction(enter, DEFAULT_KEYMAP, "files", true)).toBeNull();
    expect(findAction(enter, DEFAULT_KEYMAP, "composer", true)).toBe(
      "submitComment",
    );
    const c = {
      key: "c",
      code: "KeyC",
      metaKey: false,
      ctrlKey: false,
      altKey: false,
      shiftKey: false,
    };
    expect(findAction(c, DEFAULT_KEYMAP, "files", true)).toBe("commentLine");
  });
});

describe("resolveKeymap", () => {
  it("lays the user's bindings over the defaults", () => {
    const map = resolveKeymap({ refresh: ["Mod+KeyK"] });
    expect(map.refresh).toEqual(["Mod+KeyK"]);
    expect(map.openReview).toEqual(["Enter"]);
  });

  it("ignores unknown actions, empty lists and unreadable bindings", () => {
    const map = resolveKeymap({
      nope: ["Mod+KeyK"],
      refresh: [],
      openReview: ["junk+"],
      nextReview: "Mod+KeyK",
    });
    expect(map).toEqual(DEFAULT_KEYMAP);
    expect(resolveKeymap(undefined)).toEqual(DEFAULT_KEYMAP);
    expect(resolveKeymap("x")).toEqual(DEFAULT_KEYMAP);
  });
});

describe("findAction", () => {
  it("finds the action of its scope", () => {
    const r = ev({ key: "R", shiftKey: true });
    expect(findAction(r, DEFAULT_KEYMAP, "list", false)).toBe("refresh");
    expect(findAction(r, DEFAULT_KEYMAP, "app", false)).toBeNull();
    expect(findAction(ev({ key: "r" }), DEFAULT_KEYMAP, "list", false)).toBe(
      null,
    );
    expect(findAction(ev({ key: "r" }), DEFAULT_KEYMAP, "files", false)).toBe(
      "replyThread",
    );
    expect(findAction(ev({ key: "3" }), DEFAULT_KEYMAP, "list", false)).toBe(
      "showWaitingOnAuthor",
    );
    expect(
      findAction(ev({ key: "ArrowDown" }), DEFAULT_KEYMAP, "list", false),
    ).toBe("nextReview");
    // A modifier chord is another binding: Ctrl+3 is not list 3.
    expect(
      findAction(
        ev({ key: "3", ctrlKey: true }),
        DEFAULT_KEYMAP,
        "list",
        false,
      ),
    ).toBeNull();
  });

  it("keeps the file keys to the files scope", () => {
    expect(findAction(ev({ key: "j" }), DEFAULT_KEYMAP, "files", false)).toBe(
      "nextFile",
    );
    expect(findAction(ev({ key: "v" }), DEFAULT_KEYMAP, "files", false)).toBe(
      "toggleViewed",
    );
    expect(
      findAction(ev({ key: "j" }), DEFAULT_KEYMAP, "list", false),
    ).toBeNull();
  });

  it("leaves Settings to the native menu on macOS", () => {
    const e = ev({ key: ",", code: "Comma", metaKey: true });
    expect(findAction(e, DEFAULT_KEYMAP, "app", true)).toBeNull();
    expect(
      findAction(
        ev({ key: ",", code: "Comma", ctrlKey: true }),
        DEFAULT_KEYMAP,
        "app",
        false,
      ),
    ).toBe("openSettings");
  });
});

describe("recording", () => {
  it("turns a key press into a binding, but not a bare modifier", () => {
    expect(
      bindingFromEvent(ev({ key: "K", ctrlKey: true, shiftKey: true }), false),
    ).toEqual({ code: "KeyK", mod: true, shift: true, alt: false });
    expect(bindingFromEvent(ev({ key: "Shift", shiftKey: true }), false)).toBe(
      null,
    );
    // Ctrl on a Mac is not one of our modifiers.
    expect(bindingFromEvent(ev({ key: "k", ctrlKey: true }), true)).toBeNull();
  });

  it("refuses Escape and the fixed keys", () => {
    const b = (text: string) => parseBinding(text)!;
    expect(bindingProblem(b("Escape"), true)).toMatch(/Escape/);
    expect(bindingProblem(b("Mod+KeyA"), true)).toBe(
      "⌘A is fixed (Select all).",
    );
    expect(bindingProblem(b("Tab"), false)).toMatch(/Tab is fixed/);
    expect(bindingProblem(b("Space"), false)).toMatch(/Space is fixed/);
    expect(bindingProblem(b("Mod+KeyQ"), true)).toMatch(/Quit/);
    expect(bindingProblem(b("Mod+KeyQ"), false)).toBeNull();
  });

  it("allows bare keys, function keys and modified keys", () => {
    const b = (text: string) => parseBinding(text)!;
    expect(bindingProblem(b("KeyK"), true)).toBeNull();
    expect(bindingProblem(b("Shift+KeyK"), false)).toBeNull();
    expect(bindingProblem(b("F7"), false)).toBeNull();
    expect(bindingProblem(b("Alt+KeyK"), false)).toBeNull();
    expect(bindingProblem(b("Mod+KeyK"), true)).toBeNull();
  });

  it("finds the other action that uses a binding", () => {
    const enter = parseBinding("Enter")!;
    expect(findConflict(DEFAULT_KEYMAP, "refresh", enter, true)).toBe(
      "openReview",
    );
    expect(findConflict(DEFAULT_KEYMAP, "openReview", enter, true)).toBeNull();
    // An alias counts too.
    expect(
      findConflict(
        DEFAULT_KEYMAP,
        "openReview",
        parseBinding("Mod+KeyR")!,
        true,
      ),
    ).toBe("refresh");
    expect(
      findConflict(DEFAULT_KEYMAP, "refresh", parseBinding("Mod+KeyK")!, true),
    ).toBe("openPalette");
    expect(
      findConflict(DEFAULT_KEYMAP, "refresh", parseBinding("Mod+KeyU")!, true),
    ).toBeNull();
  });
});

describe("ownsEnter", () => {
  it("counts form inputs, links and buttons as owning their Enter", () => {
    const make = (html: string) => {
      const host = document.createElement("div");
      host.innerHTML = html;
      return host.firstElementChild;
    };
    expect(ownsEnter(make('<input type="checkbox">'))).toBe(true);
    expect(ownsEnter(make('<input type="text">'))).toBe(true);
    expect(ownsEnter(make('<a href="#x">x</a>'))).toBe(true);
    expect(ownsEnter(make("<button>x</button>"))).toBe(true);
    expect(ownsEnter(make('<div tabindex="0">x</div>'))).toBe(false);
    expect(ownsEnter(null)).toBe(false);
  });
});

describe("isBrowserShortcut", () => {
  it("catches reload, print, find and save", () => {
    const hit = (init: Partial<KeyEventLike> & { key: string }) =>
      isBrowserShortcut(ev(init), false);
    expect(hit({ key: "F5" })).toBe(true);
    expect(hit({ key: "r", ctrlKey: true })).toBe(true);
    expect(hit({ key: "R", ctrlKey: true, shiftKey: true })).toBe(true);
    expect(hit({ key: "p", ctrlKey: true })).toBe(true);
    expect(hit({ key: "f", ctrlKey: true })).toBe(true);
    expect(hit({ key: "s", ctrlKey: true })).toBe(true);
    expect(isBrowserShortcut(ev({ key: "r", metaKey: true }), true)).toBe(true);
  });

  it("leaves everything else alone", () => {
    expect(isBrowserShortcut(ev({ key: "r" }), false)).toBe(false);
    expect(isBrowserShortcut(ev({ key: "t", ctrlKey: true }), false)).toBe(
      false,
    );
    expect(
      isBrowserShortcut(
        ev({ key: "®", code: "KeyR", ctrlKey: true, altKey: true }),
        false,
      ),
    ).toBe(false);
  });
});

describe("thread keys", () => {
  it("are s to resolve and ↑/↓ for the open threads, shared with the list", () => {
    expect(DEFAULT_KEYMAP.resolveThread).toEqual(["KeyS"]);
    expect(DEFAULT_KEYMAP.nextOpenThread).toEqual(["ArrowDown"]);
    expect(DEFAULT_KEYMAP.prevOpenThread).toEqual(["ArrowUp"]);
    const down = parseBinding("ArrowDown")!;
    // Sharing ↓ with the list cursor is no conflict; with anything else it is.
    expect(findConflict(DEFAULT_KEYMAP, "nextOpenThread", down, true)).toBe(
      null,
    );
    expect(findConflict(DEFAULT_KEYMAP, "nextReview", down, true)).toBe(null);
    expect(findConflict(DEFAULT_KEYMAP, "prevOpenThread", down, true)).toBe(
      "nextReview",
    );
    expect(
      findConflict(
        DEFAULT_KEYMAP,
        "resolveThread",
        parseBinding("KeyR")!,
        true,
      ),
    ).toBe("replyThread");
    // Rebindable like the others.
    expect(resolveKeymap({ resolveThread: ["KeyQ"] }).resolveThread).toEqual([
      "KeyQ",
    ]);
  });
});
