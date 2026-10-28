/**
 * The review page's keyboard focus (M6): one focused item — the
 * Conversation, a file, a thread, an AI finding or a diff line — instead of
 * a "last clicked" per kind. The rules (what is a stop, the page order) are
 * `lib/focusModel`; this store holds the item and keeps the older per-kind
 * state in step, so the keys that read it act on the focused item:
 *
 * - focusing a thread sets `historyStore.focused` (the then/now key) and
 *   clears `aiStore.focused`; a finding the other way round; anything else
 *   clears both. A line keeps `commentStore.focus` (the comment key falls
 *   back to the line last clicked).
 * - clicks still go to those stores (`ThreadBlock`, `FindingBlock`, the
 *   diff); this store follows them, and follows j/k and tree clicks
 *   (`codeViewStore.selected`) unless the item is already in that card.
 *
 * `zone` says where Enter goes: the focused thread card after a page move,
 * the review list after ↑/↓ moved its cursor. `moveSeq` grows on every keyboard move
 * so the page (`usePageFocus`) puts the DOM focus on the item and scrolls
 * to it; `reveal` asks a folded card to open; `command` asks a thread card
 * to open/fold or start a reply (its open state is its own).
 */

import { useEffect, useRef } from "react";
import { create } from "zustand";

import type { ReviewId, ThreadId } from "../ipc/types";
import {
  cardOf,
  findingItem,
  moveFocus,
  moveToOpenThread,
  pageStops,
  sameItem,
  threadItem,
  type FocusItem,
} from "../lib/focusModel";
import { useAiStore } from "./aiStore";
import { CONVERSATION, useCodeViewStore } from "./codeViewStore";
import { useCommentStore } from "./commentStore";
import { useHistoryStore } from "./historyStore";
import { useReviewStore } from "./reviewStore";
import { useThreadStore } from "./threadStore";

export type FocusZone = "page" | "list";

export interface ThreadCommand {
  threadId: ThreadId;
  action: "toggle" | "reply";
  seq: number;
}

interface FocusState {
  reviewId: ReviewId | null;
  item: FocusItem | null;
  zone: FocusZone;
  moveSeq: number;
  reveal: { card: string; seq: number } | null;
  command: ThreadCommand | null;

  /**
   * Focus `item` on `reviewId`'s page. `move`: by keyboard — select its
   * card, open it, take the DOM focus there and scroll to it.
   */
  focusItem: (
    reviewId: ReviewId,
    item: FocusItem,
    opts?: { move?: boolean },
  ) => void;
  /** `n` / `p`: false at the end of the page or without stops. */
  move: (reviewId: ReviewId, delta: 1 | -1) => boolean;
  /**
   * ↑/↓: the next or previous thread that is not resolved, opened; false
   * at the end of the page or without one.
   */
  moveOpen: (reviewId: ReviewId, delta: 1 | -1) => boolean;
  /** j/k moved the selection: the page takes the DOM focus. */
  pageMoved: () => void;
  commandThread: (threadId: ThreadId, action: ThreadCommand["action"]) => void;
  setZone: (zone: FocusZone) => void;
}

let seq = 0;
/** Set while this store writes the other stores: their echo is ignored. */
let echo = false;

function quietly(fn: () => void) {
  echo = true;
  try {
    fn();
  } finally {
    echo = false;
  }
}

/** The review's stops as its page shows them now. */
export function currentStops(reviewId: ReviewId): FocusItem[] {
  const code = useCodeViewStore.getState().byReview[reviewId];
  const ai = useAiStore.getState();
  const enabled = !!ai.runs[reviewId]?.enabled;
  const findings = enabled
    ? (ai.byReview[reviewId]?.findings ?? []).filter(
        (f) => ai.showDismissed || !f.dismissed,
      )
    : [];
  return pageStops({
    threads: useThreadStore.getState().byReview[reviewId]?.threads ?? [],
    files: code?.files?.files ?? null,
    failed: !!code?.error,
    order: code?.order.length ? code.order : [CONVERSATION],
    since: code?.files?.sinceLastReview ?? false,
    findings,
  });
}

/** The card a thread shows in, from the stores (Conversation if no file). */
function threadFocus(reviewId: ReviewId, threadId: ThreadId, mine: boolean) {
  const t = useThreadStore
    .getState()
    .byReview[reviewId]?.threads.find((x) => x.threadId === threadId);
  const found = currentStops(reviewId).find(
    (s) => s.kind === "thread" && s.threadId === threadId,
  );
  if (found) return found;
  if (t) {
    const path = t.anchor?.pathNew ?? t.anchor?.pathOld;
    const since =
      useCodeViewStore.getState().byReview[reviewId]?.files?.sinceLastReview ??
      false;
    return threadItem(t, path ?? CONVERSATION, since);
  }
  return {
    kind: "thread" as const,
    threadId,
    mine,
    card: CONVERSATION,
    line: 0,
    side: null,
  };
}

export const useFocusStore = create<FocusState>((set, get) => ({
  reviewId: null,
  item: null,
  zone: "page",
  moveSeq: 0,
  reveal: null,
  command: null,

  focusItem: (reviewId, item, opts) => {
    const move = !!opts?.move;
    quietly(() => {
      const history = useHistoryStore.getState();
      const ai = useAiStore.getState();
      if (item.kind === "thread") {
        history.focus({ threadId: item.threadId, reviewId, mine: item.mine });
      } else if (history.focused) {
        useHistoryStore.setState({ focused: null });
      }
      if (item.kind === "finding") {
        if (ai.focused !== item.findingId) ai.focus(item.findingId);
      } else if (ai.focused) {
        ai.focus(null);
      }
      if (move) {
        const card = cardOf(item);
        const code = useCodeViewStore.getState();
        if (code.byReview[reviewId]?.selected !== card) {
          code.select(reviewId, card);
        }
      }
    });
    if (move) seq += 1;
    set({
      reviewId,
      item,
      zone: "page",
      ...(move
        ? { moveSeq: seq, reveal: { card: cardOf(item), seq } }
        : undefined),
    });
  },

  move: (reviewId, delta) => {
    const s = get();
    const current = s.reviewId === reviewId ? s.item : null;
    const next = moveFocus(currentStops(reviewId), current, delta);
    if (!next) return false;
    get().focusItem(reviewId, next, { move: true });
    return true;
  },

  moveOpen: (reviewId, delta) => {
    const s = get();
    const current = s.reviewId === reviewId ? s.item : null;
    const threads = useThreadStore.getState().byReview[reviewId]?.threads ?? [];
    const next = moveToOpenThread(
      currentStops(reviewId),
      current,
      delta,
      threads,
    );
    if (next?.kind !== "thread") return false;
    // Opened as well: a folded thread is one line, and the jump is to read it.
    quietly(() => useHistoryStore.getState().setOpen(next.threadId, true));
    get().focusItem(reviewId, next, { move: true });
    return true;
  },

  pageMoved: () => {
    seq += 1;
    set({ moveSeq: seq, zone: "page" });
  },

  commandThread: (threadId, action) => {
    seq += 1;
    set({ command: { threadId, action, seq } });
  },

  setZone: (zone) => {
    if (get().zone !== zone) set({ zone });
  },
}));

// -- Following the other stores ------------------------------------------------

function follow(reviewId: ReviewId | null, item: FocusItem) {
  if (!reviewId) return;
  const s = useFocusStore.getState();
  if (s.reviewId === reviewId && sameItem(s.item, item)) {
    // A click on a thread is a page action even when it was focused.
    s.setZone("page");
    return;
  }
  useFocusStore.setState({ reviewId, item, zone: "page" });
}

useHistoryStore.subscribe((s, prev) => {
  const f = s.focused;
  if (echo || !f || f === prev.focused) return;
  follow(f.reviewId, threadFocus(f.reviewId, f.threadId, f.mine));
});

useAiStore.subscribe((s, prev) => {
  if (echo || !s.focused || s.focused === prev.focused) return;
  const reviewId = useReviewStore.getState().selectedId;
  if (!reviewId) return;
  const f = s.byReview[reviewId]?.findings.find((x) => x.id === s.focused);
  if (!f) return;
  const since =
    useCodeViewStore.getState().byReview[reviewId]?.files?.sinceLastReview ??
    false;
  follow(reviewId, findingItem(f, since));
});

useCommentStore.subscribe((s, prev) => {
  const f = s.focus;
  if (echo || !f || f === prev.focus) return;
  quietly(() => {
    if (useHistoryStore.getState().focused) {
      useHistoryStore.setState({ focused: null });
    }
    if (useAiStore.getState().focused) useAiStore.getState().focus(null);
  });
  follow(f.reviewId, {
    kind: "line",
    path: f.path,
    side: f.side,
    line: f.line,
  });
});

useCodeViewStore.subscribe((s, prev) => {
  if (echo) return;
  const reviewId = useReviewStore.getState().selectedId;
  if (!reviewId) return;
  const selected = s.byReview[reviewId]?.selected ?? null;
  if (!selected || selected === prev.byReview[reviewId]?.selected) return;
  const cur = useFocusStore.getState();
  // A finding revealed from the AI panel selects its file after it.
  if (cur.reviewId === reviewId && cur.item && cardOf(cur.item) === selected) {
    return;
  }
  const item: FocusItem =
    selected === CONVERSATION
      ? { kind: "conversation" }
      : { kind: "file", path: selected };
  quietly(() => {
    if (useHistoryStore.getState().focused) {
      useHistoryStore.setState({ focused: null });
    }
    if (useAiStore.getState().focused) useAiStore.getState().focus(null);
  });
  useFocusStore.setState({ reviewId, item });
});

useReviewStore.subscribe((s, prev) => {
  if (s.selectedId !== prev.selectedId) {
    useFocusStore.setState({
      reviewId: s.selectedId,
      item: null,
      zone: "page",
    });
  } else if (s.cursorId !== prev.cursorId && s.cursorId !== null) {
    // ↑/↓ in the list: Enter opens the review under the cursor again.
    useFocusStore.getState().setZone("list");
  }
});

// -- Hooks ----------------------------------------------------------------------

/**
 * A thread card's focus and keyboard commands: whether it is the focused
 * item, and runs `toggle` / `reply` when the keys ask (once per request; a
 * card rebuilt by its editor does not replay an old one).
 */
export function useThreadFocus(
  threadId: ThreadId,
  handlers: { toggle: () => void; reply: () => void },
): {
  focused: boolean;
  /**
   * The card's own keys: Enter on the focused card (not a button or field
   * inside it) opens or folds it — unless ↑/↓ in the review list came last,
   * then Enter is the list's.
   */
  onKeyDown: (e: React.KeyboardEvent<HTMLElement>) => void;
} {
  const focused = useFocusStore(
    (s) => s.item?.kind === "thread" && s.item.threadId === threadId,
  );
  const command = useFocusStore((s) =>
    s.command?.threadId === threadId ? s.command : null,
  );
  const seen = useRef(command?.seq ?? 0);
  const latest = useRef(handlers);
  latest.current = handlers;
  useEffect(() => {
    if (!command || command.seq <= seen.current) return;
    seen.current = command.seq;
    latest.current[command.action]();
  }, [command]);
  const onKeyDown = (e: React.KeyboardEvent<HTMLElement>) => {
    if (
      e.key !== "Enter" ||
      e.target !== e.currentTarget ||
      e.metaKey ||
      e.ctrlKey ||
      e.altKey ||
      e.shiftKey ||
      useFocusStore.getState().zone !== "page"
    ) {
      return;
    }
    e.preventDefault();
    latest.current.toggle();
  };
  return { focused, onKeyDown };
}

/**
 * Run `open` when a keyboard move asks the card `card` to open (a folded
 * file or Conversation holding the focused item).
 */
export function useRevealCard(card: string, open: () => void) {
  const reveal = useFocusStore((s) =>
    s.reveal?.card === card ? s.reveal.seq : null,
  );
  const seen = useRef(reveal);
  const latest = useRef(open);
  latest.current = open;
  useEffect(() => {
    if (reveal !== null && reveal !== seen.current) latest.current();
    seen.current = reveal;
  }, [reveal]);
}

/**
 * Focus a thread of the review by keyboard rules (select its card, open
 * it, scroll to it): the "repeats …" link of a set-aside finding.
 */
export function focusThread(reviewId: ReviewId, threadId: ThreadId): void {
  const t = useThreadStore
    .getState()
    .byReview[reviewId]?.threads.find((x) => x.threadId === threadId);
  useFocusStore
    .getState()
    .focusItem(reviewId, threadFocus(reviewId, threadId, t?.mine ?? false), {
      move: true,
    });
}
