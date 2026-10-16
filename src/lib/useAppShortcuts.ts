/**
 * The window-wide shortcuts, from the keymap (`lib/keymap`; rebindable in
 * Settings → Shortcuts) — Iris's `useTabShortcuts` for Argus: Settings off
 * macOS (macOS has the native menu's ⌘,), and the review list keys (move,
 * open, refresh, switch lists), which are ignored while typing in a field or
 * inside a dialog.
 *
 * It also swallows the browser's own shortcuts (reload, print, find, save):
 * a desktop app must never reload its webview, wherever the focus is.
 */

import { useEffect } from "react";

import { TABS } from "./reviews";
import { useReviewStore } from "../state/reviewStore";
import {
  findAction,
  isBrowserShortcut,
  isEditableTarget,
  type ActionId,
} from "./keymap";
import { currentKeymap } from "./useKeymap";

/** Inside a dialog (Settings) the keys belong to it. */
function inDialog(target: EventTarget | null): boolean {
  return (
    target instanceof Element && target.closest('[role="dialog"]') !== null
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
      // A focused button handles its own Enter.
      if (target instanceof HTMLButtonElement) return false;
      store.openCursor();
      return true;
    case "refresh":
      void store.refresh();
      return true;
    default: {
      const tab = LIST_FOR[action];
      if (!tab) return false;
      store.setTab(tab);
      return true;
    }
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
          const action = findAction(e, keymap, "list");
          if (action && runListAction(action, e.target)) {
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
