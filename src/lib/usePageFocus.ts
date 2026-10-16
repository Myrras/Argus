/**
 * The review page side of the focus model: after a keyboard move
 * (`focusStore.moveSeq`) put the DOM focus on the focused item — a thread or
 * finding card, or a file's (the Conversation's) header button — and scroll
 * a thread or finding to the middle. Real DOM focus is what a screen reader
 * announces; the ring is the item's `data-focused` / `:focus-visible`.
 *
 * A thread or finding in a file whose diff is still loading is not in the
 * DOM yet: it is looked for a while (like the AI panel's "scroll to
 * finding"), then the card is scrolled to instead.
 */

import { useEffect } from "react";

import type { ReviewId } from "../ipc/types";
import { useFocusStore } from "../state/focusStore";
import { CONVERSATION_CARD, cardOf, type FocusItem } from "./focusModel";

const TRIES = 40;
const EVERY_MS = 50;

/** The card (`data-file-card`) of a file path or the Conversation. */
function cardElement(card: string): HTMLElement | null {
  if (card === CONVERSATION_CARD) {
    return document.querySelector<HTMLElement>(
      '[data-file-card="conversation"]',
    );
  }
  const cards = document.querySelectorAll<HTMLElement>(
    '[data-file-card="file"]',
  );
  for (const el of cards) if (el.dataset.path === card) return el;
  return null;
}

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
    let tries = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = () => {
      const el = focusElement(item);
      if (el) {
        el.focus({ preventScroll: true });
        // Files: the view already scrolls a newly selected card to its top.
        if (item.kind === "thread" || item.kind === "finding") {
          el.scrollIntoView?.({ block: "center" });
        }
        return;
      }
      tries += 1;
      if (tries < TRIES) {
        timer = setTimeout(tick, EVERY_MS);
      } else {
        cardElement(cardOf(item))?.scrollIntoView?.({ block: "start" });
      }
    };
    // After the view's own scroll to the newly selected card.
    timer = setTimeout(tick, 0);
    return () => clearTimeout(timer);
  }, [moveSeq, reviewId]);
}
