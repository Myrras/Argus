/**
 * Thread histories (M3): the then/now code and timeline per thread, as the
 * backend computes them (`review_histories`, `thread_history`).
 *
 * - A review's histories load in one call when it opens (`loadReview`),
 *   kept per review + head: reopening at the same head reads nothing.
 * - Re-read on `review:changed` (`subscribeHistoryEvents`) and whenever the
 *   thread store re-reads a review's threads with different content (a
 *   reply, a resolve): the timeline's notes come from the stored threads.
 *   Overlapping re-reads coalesce into one more call.
 * - One thread on demand (`loadThread`): a retry, or a thread the review's
 *   last read did not have yet.
 *
 * Also the then/now UI state, here because a thread card inside a diff is
 * rebuilt with its CodeMirror widget: which threads have then/now open
 * (`expanded`; unset = the default, open for the user's own threads) and the
 * thread last clicked (`focused`, for the `toggleThenNow` key).
 */

import { create } from "zustand";

import { reviewHistories, threadHistory, toIpcError } from "../ipc/commands";
import { onReviewChanged } from "../ipc/events";
import type {
  IpcError,
  ReviewId,
  StoredThread,
  ThreadHistory,
  ThreadId,
} from "../ipc/types";
import { useThreadStore } from "./threadStore";

export interface HistoryEntry {
  history: ThreadHistory | null;
  loading: boolean;
  error: IpcError | null;
}

export interface ReviewHistories {
  /** The head the last successful read was for. */
  headSha: string | null;
  loading: boolean;
  /** A read succeeded at least once. */
  loaded: boolean;
  error: IpcError | null;
}

export interface FocusedThread {
  threadId: ThreadId;
  reviewId: ReviewId;
  mine: boolean;
}

interface HistoryStoreState {
  byReview: Record<ReviewId, ReviewHistories>;
  byThread: Record<ThreadId, HistoryEntry>;
  expanded: Record<ThreadId, boolean>;
  focused: FocusedThread | null;
  /** On open: read unless already read for `headSha`. */
  loadReview: (id: ReviewId, headSha?: string | null) => Promise<void>;
  /** Read again (events); only for a review read before. */
  reloadReview: (id: ReviewId) => Promise<void>;
  loadThread: (threadId: ThreadId) => Promise<void>;
  setExpanded: (threadId: ThreadId, open: boolean) => void;
  focus: (thread: FocusedThread) => void;
  /** The `toggleThenNow` key: false when no thread of `reviewId` was clicked. */
  toggleFocused: (reviewId: ReviewId) => boolean;
}

const NO_REVIEW: ReviewHistories = {
  headSha: null,
  loading: false,
  loaded: false,
  error: null,
};
const NO_ENTRY: HistoryEntry = { history: null, loading: false, error: null };

/** The running read per review, and whether another must follow it. */
const inflight = new Map<ReviewId, Promise<void>>();
const again = new Set<ReviewId>();
/**
 * The threads (as JSON) each review's histories were last read with; unset
 * while the read ran beside a thread read.
 */
const readWith = new Map<ReviewId, string>();

function threadsOf(id: ReviewId): StoredThread[] {
  return useThreadStore.getState().byReview[id]?.threads ?? [];
}

export const useHistoryStore = create<HistoryStoreState>((set, get) => {
  const readOnce = async (id: ReviewId, head: string | null) => {
    const known = threadsOf(id);
    // Read alongside the threads' own first read (or a re-read), the
    // backend sees the same notes: the thread read that lands next is taken
    // as what this read was made with.
    if (useThreadStore.getState().byReview[id]?.loading === false) {
      readWith.set(id, JSON.stringify(known));
    } else {
      readWith.delete(id);
    }
    set((s) => {
      const byThread = { ...s.byThread };
      for (const t of known) {
        byThread[t.threadId] = {
          ...(byThread[t.threadId] ?? NO_ENTRY),
          loading: true,
        };
      }
      return {
        byThread,
        byReview: {
          ...s.byReview,
          [id]: { ...(s.byReview[id] ?? NO_REVIEW), loading: true },
        },
      };
    });
    try {
      const histories = await reviewHistories(id);
      if (!Array.isArray(histories)) throw new Error("No histories returned.");
      set((s) => {
        const byThread = { ...s.byThread };
        for (const t of known) {
          byThread[t.threadId] = {
            ...(byThread[t.threadId] ?? NO_ENTRY),
            loading: false,
          };
        }
        for (const h of histories) {
          byThread[h.threadId] = { history: h, loading: false, error: null };
        }
        return {
          byThread,
          byReview: {
            ...s.byReview,
            [id]: {
              headSha: histories[0]?.headSha ?? head,
              loading: false,
              loaded: true,
              error: null,
            },
          },
        };
      });
    } catch (e) {
      const error = toIpcError(e);
      set((s) => {
        const byThread = { ...s.byThread };
        // Keep what was read before; the cards fall back to plain notes
        // only where there is nothing.
        for (const t of known) {
          byThread[t.threadId] = {
            ...(byThread[t.threadId] ?? NO_ENTRY),
            loading: false,
            error,
          };
        }
        return {
          byThread,
          byReview: {
            ...s.byReview,
            [id]: { ...(s.byReview[id] ?? NO_REVIEW), loading: false, error },
          },
        };
      });
    }
  };

  const read = (id: ReviewId, head: string | null): Promise<void> => {
    const running = inflight.get(id);
    if (running) {
      again.add(id);
      return running;
    }
    const p = (async () => {
      do {
        again.delete(id);
        await readOnce(id, head ?? get().byReview[id]?.headSha ?? null);
      } while (again.has(id));
    })().finally(() => inflight.delete(id));
    inflight.set(id, p);
    return p;
  };

  return {
    byReview: {},
    byThread: {},
    expanded: {},
    focused: null,

    loadReview: async (id, headSha = null) => {
      const r = get().byReview[id];
      if (r?.loaded && !r.error && headSha !== null && r.headSha === headSha) {
        return;
      }
      await read(id, headSha);
    },

    reloadReview: async (id) => {
      if (get().byReview[id]) await read(id, null);
    },

    loadThread: async (threadId) => {
      set((s) => ({
        byThread: {
          ...s.byThread,
          [threadId]: { ...(s.byThread[threadId] ?? NO_ENTRY), loading: true },
        },
      }));
      try {
        const history = await threadHistory(threadId);
        if (!history || typeof history !== "object") {
          throw new Error("No history returned.");
        }
        set((s) => ({
          byThread: {
            ...s.byThread,
            [threadId]: { history, loading: false, error: null },
          },
        }));
      } catch (e) {
        set((s) => ({
          byThread: {
            ...s.byThread,
            [threadId]: {
              ...(s.byThread[threadId] ?? NO_ENTRY),
              loading: false,
              error: toIpcError(e),
            },
          },
        }));
      }
    },

    setExpanded: (threadId, open) =>
      set((s) => ({ expanded: { ...s.expanded, [threadId]: open } })),

    focus: (thread) => {
      const f = get().focused;
      if (f?.threadId !== thread.threadId || f.mine !== thread.mine) {
        set({ focused: thread });
      }
    },

    toggleFocused: (reviewId) => {
      const f = get().focused;
      if (!f || f.reviewId !== reviewId) return false;
      const open = get().expanded[f.threadId] ?? f.mine;
      get().setExpanded(f.threadId, !open);
      return true;
    },
  };
});

/** Whether a thread's then/now is open (the toggle, else open when mine). */
export function thenNowOpen(
  expanded: Record<ThreadId, boolean>,
  t: Pick<StoredThread, "threadId" | "mine">,
): boolean {
  return expanded[t.threadId] ?? t.mine;
}

// A thread re-read whose content differs from what the histories were read
// with (a reply, a resolve, a new thread) re-reads them: the timeline's
// notes come from the stored threads.
useThreadStore.subscribe((s, prev) => {
  const reviews = useHistoryStore.getState().byReview;
  for (const id of Object.keys(reviews)) {
    const now = s.byReview[id];
    if (!now || now.loading || now.threads === prev.byReview[id]?.threads) {
      continue;
    }
    const read = readWith.get(id);
    const threads = JSON.stringify(now.threads);
    if (read === undefined) readWith.set(id, threads);
    else if (read !== threads) void useHistoryStore.getState().reloadReview(id);
  }
});

/** Follow `review:changed`; resolves to an unsubscribe. */
export function subscribeHistoryEvents(): Promise<() => void> {
  return onReviewChanged(
    (e) => void useHistoryStore.getState().reloadReview(e.reviewId),
  );
}

/** Tests: forget the reads in flight and what they were read with. */
export function clearHistoryReads(): void {
  inflight.clear();
  again.clear();
  readWith.clear();
}
