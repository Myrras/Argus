/**
 * The keyboard map (Iris's `lib/keymap`): named actions with their default
 * bindings, a matcher for key events, and the display format.
 *
 * A binding is a string such as `"Mod+Shift+KeyN"`: `Mod` is Cmd on macOS and
 * Ctrl elsewhere, the last part is the key. Letters and digits are matched by
 * the character they type (so Dvorak, AZERTY and Cyrillic layouts keep Cmd+P
 * on P), everything else and every Option/Alt combination by the physical
 * `code` (Option+S types "ß" on a Mac, so its `key` says nothing).
 *
 * Settings → Shortcuts stores only the bindings the user changed
 * (`settingsStore.shortcuts`); `resolveKeymap` lays them over the defaults,
 * so an action added later still gets its default. The clipboard and undo
 * keys stay fixed (`RESERVED`).
 *
 * Unlike Iris, a bare key may be bound: Argus has no type-to-select for it to
 * fight, and its list keys (`1`–`4`, the arrows) and the review page's
 * focus keys (`n`/`p`, `r`, `e`, `d`, M6; `s` resolves) are bare by default.
 * ↑/↓ are both the list's and the review page's (next/previous open
 * thread): where the focus is decides (`SHARED_BY_CONTEXT`).
 * `list` actions only fire outside fields and dialogs (`useAppShortcuts`);
 * `files` actions likewise, and only while a review is open; `composer`
 * actions only inside a comment composer (`CommentComposer`).
 *
 * Pure rules, no React: `useAppShortcuts.ts` dispatches with `findAction`.
 */

import { isMac } from "./platform";

export type ActionId =
  | "nextReview"
  | "prevReview"
  | "openReview"
  | "refresh"
  | "showNeedsReview"
  | "showNeedsReReview"
  | "showWaitingOnAuthor"
  | "showClosedWithoutApproval"
  | "nextFile"
  | "prevFile"
  | "toggleViewed"
  | "commentLine"
  | "submitComment"
  | "cancelComment"
  | "openSettings"
  | "toggleThenNow"
  | "toggleFindings"
  | "toggleFindingSelected"
  | "nextItem"
  | "prevItem"
  | "toggleThread"
  | "replyThread"
  | "editFinding"
  | "dismissFinding"
  | "resolveThread"
  | "nextOpenThread"
  | "prevOpenThread"
  | "openPalette"
  | "showShortcuts"
  // Open in editor
  | "openInEditor"
  | "showInFinder";

/**
 * `list`: the review list, outside fields and dialogs; `files`: the review
 * view, likewise; `composer`: inside a comment composer's text field only
 * (the composer matches them itself); `app`: window-wide.
 */
export type ActionScope = "list" | "files" | "composer" | "app";

export type ActionGroup = "Navigate" | "Lists" | "Files" | "Comments" | "View";

export interface ActionDef {
  id: ActionId;
  label: string;
  group: ActionGroup;
  scope: ActionScope;
  defaults: string[];
  /** What else to know, shown under the row. */
  note?: string;
  /** macOS only: the native menu owns it, so it is neither bound nor listed. */
  windowsLinuxOnly?: boolean;
  /** Listed but not rebindable (Escape: it cancels everywhere). */
  fixed?: boolean;
  /** Held down, it repeats (moves); every other action acts once a press. */
  repeats?: boolean;
}

export const ACTIONS: ActionDef[] = [
  {
    id: "nextReview",
    label: "Next review",
    group: "Navigate",
    scope: "list",
    defaults: ["ArrowDown"],
    repeats: true,
  },
  {
    id: "prevReview",
    label: "Previous review",
    group: "Navigate",
    scope: "list",
    defaults: ["ArrowUp"],
    repeats: true,
  },
  {
    id: "openReview",
    label: "Open review",
    group: "Navigate",
    scope: "list",
    defaults: ["Enter"],
    note: "Opens the review under the cursor.",
  },
  {
    id: "refresh",
    label: "Refresh",
    group: "Navigate",
    scope: "list",
    // Bare R replies to the focused thread (M6); refresh moved to ⇧R.
    defaults: ["Shift+KeyR", "Mod+KeyR"],
  },
  {
    id: "showNeedsReview",
    label: "Needs review",
    group: "Lists",
    scope: "list",
    defaults: ["Digit1"],
  },
  {
    id: "showNeedsReReview",
    label: "Needs re-review",
    group: "Lists",
    scope: "list",
    defaults: ["Digit2"],
  },
  {
    id: "showWaitingOnAuthor",
    label: "Waiting on author",
    group: "Lists",
    scope: "list",
    defaults: ["Digit3"],
  },
  {
    id: "showClosedWithoutApproval",
    label: "Closed without my approval",
    group: "Lists",
    scope: "list",
    defaults: ["Digit4"],
  },
  {
    id: "nextFile",
    label: "Next file",
    group: "Files",
    scope: "files",
    defaults: ["KeyJ"],
    repeats: true,
    note: "In a review: the Conversation, then each file.",
  },
  {
    id: "prevFile",
    label: "Previous file",
    group: "Files",
    scope: "files",
    defaults: ["KeyK"],
    repeats: true,
    note: "In a review: the Conversation, then each file.",
  },
  {
    id: "toggleViewed",
    label: "Mark file viewed",
    group: "Files",
    scope: "files",
    defaults: ["KeyV"],
    note: "Toggles the selected file's Viewed mark.",
  },
  {
    id: "commentLine",
    label: "Comment on line",
    group: "Comments",
    scope: "files",
    defaults: ["KeyC"],
    note: "On the focused thread's or finding's line, else the line you last clicked in a diff.",
  },
  {
    id: "submitComment",
    label: "Submit comment",
    group: "Comments",
    scope: "composer",
    defaults: ["Mod+Enter"],
    note: "In a comment: Add to review (a draft), Reply, or Save.",
  },
  {
    id: "cancelComment",
    label: "Cancel comment",
    group: "Comments",
    scope: "composer",
    defaults: ["Escape"],
    fixed: true,
    note: "Closes the comment without saving it.",
  },
  {
    id: "openSettings",
    label: "Open Settings",
    group: "View",
    scope: "app",
    defaults: ["Mod+Comma"],
    windowsLinuxOnly: true,
  },
  {
    id: "toggleThenNow",
    label: "Toggle then/now",
    group: "Comments",
    scope: "files",
    defaults: ["KeyH"],
    note: "Shows or hides the code then and now of the focused thread.",
  },
  {
    id: "toggleFindings",
    label: "Show or hide the AI findings",
    group: "View",
    scope: "files",
    defaults: ["KeyA"],
    note: "In a review with the AI pass on: the AI panel at the right.",
  },
  {
    id: "toggleFindingSelected",
    label: "Select finding for posting",
    group: "Comments",
    scope: "files",
    defaults: ["KeyX"],
    note: "Toggles the focused AI finding. Posting stays a button.",
  },
  {
    id: "nextItem",
    label: "Next thread or finding",
    group: "Files",
    scope: "files",
    defaults: ["KeyN"],
    repeats: true,
    note: "In a review: moves the focus down the page, in page order.",
  },
  {
    id: "prevItem",
    label: "Previous thread or finding",
    group: "Files",
    scope: "files",
    defaults: ["KeyP"],
    repeats: true,
    note: "In a review: moves the focus up the page.",
  },
  {
    id: "toggleThread",
    label: "Open or fold thread",
    group: "Comments",
    scope: "files",
    defaults: ["KeyT"],
    note: "On the focused thread. Enter on it does the same (after ↑/↓ in the list, Enter opens that review).",
  },
  {
    id: "replyThread",
    label: "Reply to thread",
    group: "Comments",
    scope: "files",
    defaults: ["KeyR"],
    note: "Opens a reply on the focused thread. It posts only with Reply or ⌘↩.",
  },
  {
    id: "editFinding",
    label: "Edit finding",
    group: "Comments",
    scope: "files",
    defaults: ["KeyE"],
    note: "Edits the focused AI finding's text. Nothing is posted.",
  },
  {
    id: "dismissFinding",
    label: "Dismiss finding",
    group: "Comments",
    scope: "files",
    defaults: ["KeyD"],
    note: "Dismisses (or undismisses) the focused AI finding.",
  },
  {
    id: "resolveThread",
    label: "Resolve or reopen thread",
    group: "Comments",
    scope: "files",
    defaults: ["KeyS"],
    note: "Resolves the focused thread on its platform, or reopens a resolved one.",
  },
  {
    id: "nextOpenThread",
    label: "Next open thread",
    group: "Files",
    scope: "files",
    defaults: ["ArrowDown"],
    repeats: true,
    note: "In a review: the next unresolved thread in page order. In the review list ↓ moves the cursor; in the code it picks a line.",
  },
  {
    id: "prevOpenThread",
    label: "Previous open thread",
    group: "Files",
    scope: "files",
    defaults: ["ArrowUp"],
    repeats: true,
    note: "In a review: the previous unresolved thread. In the review list ↑ moves the cursor; in the code it picks a line.",
  },
  {
    id: "openPalette",
    label: "Command palette",
    group: "View",
    scope: "list",
    defaults: ["Mod+KeyK"],
    note: "Every action, Go to review… and Go to file…",
  },
  {
    id: "showShortcuts",
    label: "Keyboard shortcuts",
    group: "View",
    scope: "list",
    defaults: ["Shift+Slash"],
    note: "Lists the shortcuts that work where you are.",
  },
  // -- Open in editor (Iris's keys: E there, O here, as E edits findings) --
  {
    id: "openInEditor",
    label: "Open in editor",
    group: "Files",
    scope: "files",
    defaults: ["KeyO"],
    note: "Opens the focused file from your local clone (Settings → Editors).",
  },
  {
    id: "showInFinder",
    label: "Show in Finder / Explorer",
    group: "Files",
    scope: "files",
    defaults: ["Mod+Alt+KeyR"],
    note: "Shows the focused file of your local clone in the file manager.",
  },
];

export const GROUPS: ActionGroup[] = [
  "Navigate",
  "Lists",
  "Files",
  "Comments",
  "View",
];

const BY_ID = new Map(ACTIONS.map((a) => [a.id, a]));

/**
 * Pairs of actions that may share a binding: where the focus is decides
 * which one a press is (`keyboardFlow.onReviewPage`): ↑/↓ move the review
 * list's cursor from the sidebar and jump between open threads on the
 * review page.
 */
const SHARED_BY_CONTEXT: [ActionId, ActionId][] = [
  ["nextReview", "nextOpenThread"],
  ["prevReview", "prevOpenThread"],
];

/** `a` and `b` may hold the same binding (see `SHARED_BY_CONTEXT`). */
export function sharesByContext(a: ActionId, b: ActionId): boolean {
  return SHARED_BY_CONTEXT.some(
    ([x, y]) => (x === a && y === b) || (x === b && y === a),
  );
}

export function actionDef(id: ActionId): ActionDef {
  return BY_ID.get(id)!;
}

/** The actions shown and bound on this platform. */
export function activeActions(mac: boolean = isMac): ActionDef[] {
  return ACTIONS.filter((a) => !(mac && a.windowsLinuxOnly));
}

// -- Bindings -----------------------------------------------------------------

export interface Binding {
  /** `KeyP`, `Digit1`, `F2`, `Delete`, `BracketLeft`, … */
  code: string;
  mod: boolean;
  shift: boolean;
  alt: boolean;
}

const MODIFIER_KEYS = new Set(["Meta", "Control", "Alt", "Shift", "AltGraph"]);

export function parseBinding(text: string): Binding | null {
  const parts = text.split("+");
  const code = parts.pop();
  if (!code) return null;
  const b: Binding = { code, mod: false, shift: false, alt: false };
  for (const part of parts) {
    if (part === "Mod") b.mod = true;
    else if (part === "Shift") b.shift = true;
    else if (part === "Alt") b.alt = true;
    else return null;
  }
  return /^[A-Za-z][A-Za-z0-9]*$/.test(code) ? b : null;
}

export function serializeBinding(b: Binding): string {
  return [
    ...(b.mod ? ["Mod"] : []),
    ...(b.alt ? ["Alt"] : []),
    ...(b.shift ? ["Shift"] : []),
    b.code,
  ].join("+");
}

/** The parts of a key event the keymap reads (DOM and React events fit). */
export interface KeyEventLike {
  key: string;
  code: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

/** Keys whose `code` the browser may leave empty (synthetic events). */
const CODE_BY_KEY: Record<string, string> = {
  " ": "Space",
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
  ";": "Semicolon",
  "'": "Quote",
  "[": "BracketLeft",
  "]": "BracketRight",
  "\\": "Backslash",
  "-": "Minus",
  "=": "Equal",
  "`": "Backquote",
};

/** The key of an event as binding code, or null for a bare modifier. */
export function codeOf(e: KeyEventLike): string | null {
  if (MODIFIER_KEYS.has(e.key)) return null;
  // user-event and some IMEs report "Unknown" or nothing.
  const code = e.code && e.code !== "Unknown" ? e.code : "";
  // Option changes the character (Option+S → "ß"): trust the physical key.
  if (!e.altKey || !code) {
    if (/^[a-z]$/i.test(e.key)) return `Key${e.key.toUpperCase()}`;
    // "?" wherever the layout puts it (Shift+ß on German) is Shift+/.
    if (e.key === "?") return "Slash";
    if (/^[0-9]$/.test(e.key) && !e.shiftKey) return `Digit${e.key}`;
  }
  if (code) return code;
  if (e.key.length > 1 && e.key !== "Dead" && e.key !== "Unidentified") {
    return e.key;
  }
  return CODE_BY_KEY[e.key] ?? null;
}

function modifiers(e: KeyEventLike, mac: boolean) {
  return {
    mod: mac ? e.metaKey : e.ctrlKey,
    // The other one (Ctrl on a Mac, the Windows key elsewhere) is never part
    // of a binding, so holding it never matches.
    stray: mac ? e.ctrlKey : e.metaKey,
  };
}

export function matches(
  e: KeyEventLike,
  binding: string | Binding,
  mac: boolean = isMac,
): boolean {
  const b = typeof binding === "string" ? parseBinding(binding) : binding;
  if (!b) return false;
  const { mod, stray } = modifiers(e, mac);
  return (
    !stray &&
    mod === b.mod &&
    e.shiftKey === b.shift &&
    e.altKey === b.alt &&
    codeOf(e) === b.code
  );
}

/** The binding a key press spells out, or null for a bare modifier. */
export function bindingFromEvent(
  e: KeyEventLike,
  mac: boolean = isMac,
): Binding | null {
  const code = codeOf(e);
  if (!code) return null;
  const { mod, stray } = modifiers(e, mac);
  // Ctrl on a Mac (or the Windows key) is not one of our modifiers.
  if (stray) return null;
  return { code, mod, shift: e.shiftKey, alt: e.altKey };
}

// -- Keymap -------------------------------------------------------------------

export type Keymap = Record<ActionId, string[]>;

/** What the user changed: the whole list of bindings for an action. */
export type ShortcutOverrides = Partial<Record<ActionId, string[]>>;

export const DEFAULT_KEYMAP = Object.fromEntries(
  ACTIONS.map((a) => [a.id, a.defaults]),
) as Keymap;

/**
 * The defaults with the user's changes laid over them. Unknown actions and
 * unreadable or empty lists are dropped, so a stale blob cannot unbind
 * anything.
 */
export function resolveKeymap(overrides?: unknown): Keymap {
  const saved = (overrides ?? {}) as Record<string, unknown>;
  const map = { ...DEFAULT_KEYMAP };
  for (const a of ACTIONS) {
    const list = saved[a.id];
    if (
      !a.fixed &&
      Array.isArray(list) &&
      list.length > 0 &&
      list.every((b) => typeof b === "string" && parseBinding(b) !== null)
    ) {
      map[a.id] = list as string[];
    }
  }
  return map;
}

/** The first action of `scope` whose binding the event matches. */
export function findAction(
  e: KeyEventLike,
  keymap: Keymap,
  scope: ActionScope,
  mac: boolean = isMac,
): ActionId | null {
  for (const a of activeActions(mac)) {
    if (a.scope !== scope) continue;
    if (keymap[a.id].some((b) => matches(e, b, mac))) return a.id;
  }
  return null;
}

// -- Rules for what can be bound ---------------------------------------------

/** Keys that are not rebindable, with what they do (Mod = Cmd / Ctrl). */
const RESERVED: [string, string][] = [
  ["Mod+KeyA", "Select all"],
  ["Mod+KeyC", "Copy"],
  ["Mod+KeyV", "Paste"],
  ["Mod+KeyX", "Cut"],
  ["Mod+KeyZ", "Undo"],
  ["Mod+Shift+KeyZ", "Redo"],
  ["Tab", "Move the focus"],
  ["Shift+Tab", "Move the focus"],
  ["Space", "Press the focused button"],
];

const RESERVED_MAC: [string, string][] = [
  ["Mod+Comma", "Settings"],
  ["Mod+KeyQ", "Quit"],
  ["Mod+KeyH", "Hide"],
  ["Mod+Alt+KeyH", "Hide others"],
  ["Mod+KeyM", "Minimize"],
];

const RESERVED_OTHER: [string, string][] = [["Alt+F4", "Close the window"]];

/**
 * Why a binding cannot be used at all, or null. Escape closes dialogs; the
 * reserved keys keep their usual meaning. Bare keys are fine (see the module
 * comment).
 */
export function bindingProblem(
  b: Binding,
  mac: boolean = isMac,
): string | null {
  if (b.code === "Escape") return "Escape closes dialogs and cannot be bound.";
  const text = serializeBinding(b);
  const reserved = [...RESERVED, ...(mac ? RESERVED_MAC : RESERVED_OTHER)].find(
    ([r]) => r === text,
  );
  if (reserved) {
    return `${formatBinding(b, mac)} is fixed (${reserved[1]}).`;
  }
  return null;
}

/** The action (other than `action`) that already uses the binding. */
export function findConflict(
  keymap: Keymap,
  action: ActionId,
  binding: Binding,
  mac: boolean = isMac,
): ActionId | null {
  const text = serializeBinding(binding);
  for (const a of activeActions(mac)) {
    if (a.id === action || sharesByContext(a.id, action)) continue;
    if (keymap[a.id].includes(text)) return a.id;
  }
  return null;
}

// -- Display ------------------------------------------------------------------

const SYMBOLS: Record<string, string> = {
  Comma: ",",
  Period: ".",
  Slash: "/",
  Semicolon: ";",
  Quote: "'",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Minus: "-",
  Equal: "=",
  Backquote: "`",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
};

function keyLabel(code: string, mac: boolean): string {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (code in SYMBOLS) return SYMBOLS[code];
  if (mac && code === "Backspace") return "⌫";
  if (mac && code === "Delete") return "⌦";
  if (mac && code === "Enter") return "↩";
  return code;
}

/** Keys shown as the character Shift makes of them (US layout). */
const SHIFTED: Record<string, string> = { Slash: "?" };

/** `⇧⌘G` on a Mac, `Ctrl+Shift+G` elsewhere; Shift+/ as `?`. */
export function formatBinding(
  binding: string | Binding,
  mac: boolean = isMac,
): string {
  const parsed = typeof binding === "string" ? parseBinding(binding) : binding;
  if (!parsed) return "";
  const b =
    parsed.shift && !parsed.mod && !parsed.alt && parsed.code in SHIFTED
      ? { ...parsed, shift: false }
      : parsed;
  const key = b !== parsed ? SHIFTED[parsed.code] : keyLabel(b.code, mac);
  if (mac) {
    return `${b.alt ? "⌥" : ""}${b.shift ? "⇧" : ""}${b.mod ? "⌘" : ""}${key}`;
  }
  return [
    ...(b.mod ? ["Ctrl"] : []),
    ...(b.alt ? ["Alt"] : []),
    ...(b.shift ? ["Shift"] : []),
    key,
  ].join("+");
}

/** The action's first binding for a tooltip or menu hint; "" if unbound. */
export function shortcutHint(
  keymap: Keymap,
  action: ActionId,
  mac: boolean = isMac,
): string {
  const [first] = keymap[action];
  return first ? formatBinding(first, mac) : "";
}

/** `"Refresh"` + `"⌘R"` → `"Refresh (⌘R)"`. */
export function withHint(
  title: string,
  keymap: Keymap,
  action: ActionId,
  mac: boolean = isMac,
): string {
  const hint = shortcutHint(keymap, action, mac);
  return hint ? `${title} (${hint})` : title;
}

// -- Event helpers --------------------------------------------------------------

/** Inputs that take no typing: a click on one must not mute the keys. */
const KEYLESS_INPUTS = new Set([
  "checkbox",
  "radio",
  "button",
  "submit",
  "reset",
  "image",
  "color",
  "file",
]);

/** Typing in a field never triggers the list shortcuts. */
export function isEditableTarget(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  if (el.tagName === "INPUT") {
    return !KEYLESS_INPUTS.has((el as HTMLInputElement).type);
  }
  return (
    el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable
  );
}

/**
 * Elements whose own Enter does something (follow, toggle, press, submit),
 * or that a stray Enter must not leave (a checkbox: the list's Enter would
 * open another review).
 */
const OWN_ENTER =
  'button, a[href], summary, input, select, textarea, [role="button"], [role="link"], [role="menuitem"], [role="tab"], [role="option"], [role="checkbox"], [role="switch"]';

/** A focused control that handles its own Enter: the shortcut leaves it. */
export function ownsEnter(target: EventTarget | null): boolean {
  return target instanceof Element && target.matches(OWN_ENTER);
}

/**
 * Browser shortcuts a desktop app must never act on: reload (F5, Cmd/Ctrl+R,
 * also with Shift), print, find-in-page, save and "find again". Dev builds of
 * Tauri / WebKit reload on Cmd+R, WebView2 prints on Ctrl+P. Swallowed even
 * when unbound, so rebinding Refresh away from Cmd+R does not bring the
 * reload back.
 */
export function isBrowserShortcut(
  e: KeyEventLike,
  mac: boolean = isMac,
): boolean {
  if (e.key === "F5") return true;
  const { mod } = modifiers(e, mac);
  if (!mod || e.altKey) return false;
  return ["KeyR", "KeyP", "KeyF", "KeyG", "KeyS"].includes(codeOf(e) ?? "");
}
