/**
 * The review page side of the focus model: after a keyboard move
 * (`focusStore.moveSeq`) put the DOM focus on the focused item — a thread or
 * finding card, or a file's (the Conversation's) header button — and, for a
 * thread or finding, scroll its own card to the top of the page column
 * (`lib/revealItem`: it loads the file, asks the file's editor to render
 * the line, and waits for the card). Real DOM focus is what a screen reader
 * announces; the ring is the item's `data-focused` / `:focus-visible`.
 */

import { useEffect } from "react";

import type { ReviewId } from "../ipc/types";
import { prefetchFile } from "../state/codeViewStore";
import { useFocusStore } from "../state/focusStore";
import { cardOf, type FocusItem } from "./focusModel";
import { cardElement, pageRevealEnv, revealItem } from "./revealItem";

/** The element that takes the DOM focus for `item`, if rendered. */
export function focusElement(item: FocusItem): HTMLElement | null {
  switch (item.kind) {
    case "thread":
      return document.getElementById(`thread-${item.threadId}`);
    case "finding":
      return document.getElementById(`finding-${item.findingId}`);
    case "conversation":
    case "file":
      return (
        cardElement(cardOf(item))?.querySelector<HTMLElement>(
          "[data-focus-target]",
        ) ?? null
      );
    default:
      return null;
  }
}

export function usePageFocus(reviewId: ReviewId) {
  const moveSeq = useFocusStore((s) => s.moveSeq);
  useEffect(() => {
    if (moveSeq === 0) return;
    const { item, reviewId: at } = useFocusStore.getState();
    if (!item || at !== reviewId || item.kind === "line") return;
    if (item.kind === "conversation" || item.kind === "file") {
      // The view already scrolls a newly selected card to its top; after it.
      const timer = setTimeout(
        () => focusElement(item)?.focus({ preventScroll: true }),
        0,
      );
      return () => clearTimeout(timer);
    }
    let cancel: (() => void) | null = null;
    // After the view's own scroll to the newly selected card.
    const timer = setTimeout(() => {
      cancel = revealItem(
        item,
        pageRevealEnv(item, {
          element: focusElement,
          load: (path) => prefetchFile(reviewId, path),
        }),
      );
    }, 0);
    return () => {
      clearTimeout(timer);
      cancel?.();
    };
  }, [moveSeq, reviewId]);
}
