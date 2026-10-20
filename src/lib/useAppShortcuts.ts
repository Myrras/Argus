/**
 * The window-wide shortcuts, from the keymap (`lib/keymap`; rebindable in
 * Settings → Shortcuts) — Iris's `useTabShortcuts` for Argus: Settings off
 * macOS (macOS has the native menu's ⌘,), the review list keys (move, open,
 * refresh, switch lists) and, while a review is open, the review view's keys
 * (next/previous stop — Conversation, then the files — toggle viewed, and
 * comment on the focused line, which only opens a composer: nothing posts
 * from a shortcut outside a composer; the then/now of the thread last
 * clicked; with the AI pass on, toggle the AI panel and the last-clicked
 * finding's selection — selecting, not posting).
 * M6 (`keyboardFlow`): `n`/`p` move the page focus, and the then/now,
 * selection, comment, Enter/reply/edit/dismiss keys act on the focused item;
 * ⌘K opens the command palette, ? the shortcut help. `runShortcut` runs an
 * action as its key would (the palette).
 * Those are ignored while typing in a field or inside a dialog (also while
 * a modal is open, wherever the focus is).
 *
 * It also swallows the browser's own shortcuts (reload, print, find, save):
 * a desktop app must never reload its webview, wherever the focus is.
 */

import { useEffect } from "react";

import { TABS } from "./reviews";
import { useAiStore } from "../state/aiStore";
import { useCodeViewStore } from "../state/codeViewStore";
import { useLayoutStore } from "../state/layoutStore";
import { useCommentStore } from "../state/commentStore";
import { useFocusStore } from "../state/focusStore";
import { useHistoryStore } from "../state/historyStore";
import { useReviewStore } from "../state/reviewStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import {
  commentOnFocusedItem,
  runFlowAction,
  runOverlayAction,
} from "./keyboardFlow";
import {
  actionDef,
  findAction,
  isBrowserShortcut,
  isEditableTarget,
  ownsEnter,
  type ActionId,
} from "./keymap";
import { runLocalFileAction } from "./localFiles";
import { currentKeymap } from "./useKeymap";

/** Inside a dialog (Settings) the keys belong to it. */
function inDialog(target: EventTarget | null): boolean {
  return (
    (target instanceof Element && target.closest('[role="dialog"]') !== null) ||
    // An open modal owns the keys even when the focus fell out of it (not
    // one animating closed).
    document.querySelector(
      '[role="dialog"][aria-modal="true"]:not([data-state="closed"])',
    ) !== null
  );
}

const LIST_FOR: Partial<Record<ActionId, (typeof TABS)[number]["id"]>> = {
  showNeedsReview: "needsReview",
  showNeedsReReview: "needsReReview",
  showWaitingOnAuthor: "waitingOnAuthor",
  showClosedWithoutApproval: "closedWithoutApproval",
};

/** Run a list action; false when the event is left to its target. */
function runListAction(action: ActionId, target: EventTarget | null): boolean {
  const store = useReviewStore.getState();
  switch (action) {
    case "nextReview":
      store.moveCursor(1);
      return true;
    case "prevReview":
      store.moveCursor(-1);
      return true;
    case "openReview":
      // A focused button or link handles its own Enter; without a cursor
      // there is nothing to open.
      if (ownsEnter(target) || !store.cursorId) return false;
      store.openCursor();
      return true;
    case "refresh":
      void store.refresh();
      return true;
    default: {
      const tab = LIST_FOR[action];
      if (!tab) return runOverlayAction(action) ?? false;
      store.setTab(tab);
      return true;
    }
  }
}

/** Run a review view action; false when no review is open. */
function runFilesAction(
  action: ActionId,
  target: EventTarget | null,
  key: string | null = null,
): boolean {
  const review = useReviewStore.getState().selectedId;
  if (!review) return false;
  const code = useCodeViewStore.getState();
  switch (action) {
    case "nextFile":
    case "prevFile":
      code.move(review, action === "nextFile" ? 1 : -1);
      useFocusStore.getState().pageMoved();
      return true;
    case "toggleViewed":
      void code.toggleSelectedViewed(review);
      return true;
    case "commentLine":
      // The focused thread's or finding's line, else the line last clicked
      // in a diff; nothing to do without one.
      return (
        commentOnFocusedItem(review) ||
        useCommentStore.getState().commentOnFocus(review)
      );
    case "toggleThenNow":
      // The thread last clicked; nothing to do without one.
      return useHistoryStore.getState().toggleFocused(review);
    case "toggleFindings":
      if (!useAiStore.getState().runs[review]?.enabled) return false;
      useLayoutStore.getState().toggleAiPanel();
      return true;
    case "toggleFindingSelected":
      if (!useAiStore.getState().runs[review]?.enabled) return false;
      return useAiStore.getState().toggleFocused(review);
    default:
      return (
        runFlowAction(action, review, target, key) ??
        runLocalFileAction(action, review) ??
        false
      );
  }
}

/**
 * Run `action` as its key would, outside any key event (the command
 * palette); false when there is nothing to act on.
 */
export function runShortcut(action: ActionId): boolean {
  switch (actionDef(action).scope) {
    case "files":
      return runFilesAction(action, null);
    case "list":
      return runListAction(action, null);
    case "app":
      if (action !== "openSettings") return false;
      useSettingsRequestStore.getState().open("general");
      return true;
    default:
      return false;
  }
}

export function useAppShortcuts(options?: { onOpenSettings?: () => void }) {
  const onOpenSettings = options?.onOpenSettings;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.defaultPrevented) {
        const keymap = currentKeymap();
        // Settings opens from a field too, as the native ⌘, does.
        if (findAction(e, keymap, "app") === "openSettings" && onOpenSettings) {
          e.preventDefault();
          onOpenSettings();
          return;
        }
        if (!isEditableTarget(e.target) && !inDialog(e.target)) {
          // A held key repeats the moves only: a toggle (viewed, dismiss)
          // would flap and write on every repeat.
          const once = (a: ActionId) => !e.repeat || !!actionDef(a).repeats;
          const fileAction = findAction(e, keymap, "files");
          if (
            fileAction &&
            once(fileAction) &&
            runFilesAction(fileAction, e.target, e.key)
          ) {
            e.preventDefault();
            return;
          }
          const action = findAction(e, keymap, "list");
          if (action && once(action) && runListAction(action, e.target)) {
            e.preventDefault();
            return;
          }
        }
      }
      if (isBrowserShortcut(e)) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onOpenSettings]);
}
