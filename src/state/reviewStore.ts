/**
 * Review list state: the active reviews, the selected tab and review, and
 * the last refresh. Mirrors the backend — every action calls a typed IPC
 * wrapper, then re-reads; the refresh events (`subscribeReviewEvents`) keep
 * it current between user actions.
 *
 * Refresh errors are not `lastError`: they live in `lastRefresh.errors`, per
 * provider, and never block the list.
 */

import { create } from "zustand";

import {
  getReview,
  lastRefresh,
  listReviews,
  markReviewed,
  refreshReviews,
  toIpcError,
} from "../ipc/commands";
import { onReviewChanged, onReviewsRefreshed } from "../ipc/events";
import type {
  IpcError,
  RefreshReport,
  ReviewId,
  ReviewRecord,
} from "../ipc/types";
import { reviewsInTab, type ReviewTab } from "../lib/reviews";

interface ReviewStoreState {
  reviews: ReviewRecord[];
  /** The first list call finished (success or not). */
  loaded: boolean;
  tab: ReviewTab;
  selectedId: ReviewId | null;
  /** The keyboard cursor in the current tab (↑/↓); Enter opens it. */
  cursorId: ReviewId | null;
  /** A manual refresh is running. */
  refreshing: boolean;
  /** The review whose round is being marked reviewed. */
  marking: ReviewId | null;
  lastRefresh: RefreshReport | null;
  /** The last failed IPC call (not a refresh error). */
  lastError: IpcError | null;
  /** Initial load: the list plus the last refresh the backend saw. */
  load: () => Promise<void>;
  reloadList: () => Promise<void>;
  /** Refresh every provider now (the button and `r`). */
  refresh: () => Promise<void>;
  markReviewed: (id: ReviewId) => Promise<void>;
  setTab: (tab: ReviewTab) => void;
  /** Open a review in the detail pane (moves the cursor there too). */
  select: (id: ReviewId | null) => void;
  /** Move the cursor by `delta` within the current tab, clamped. */
  moveCursor: (delta: number) => void;
  /** Open the review under the cursor. */
  openCursor: () => void;
  /** `review:changed`: re-read that one review in place. */
  applyChanged: (id: ReviewId) => Promise<void>;
  /** `reviews:refreshed`: keep the report and re-read the list. */
  applyRefreshed: (report: RefreshReport) => Promise<void>;
}

export const useReviewStore = create<ReviewStoreState>((set, get) => ({
  reviews: [],
  loaded: false,
  tab: "needsReview",
  selectedId: null,
  cursorId: null,
  refreshing: false,
  marking: null,
  lastRefresh: null,
  lastError: null,

  load: async () => {
    try {
      const [reviews, report] = await Promise.all([
        listReviews(),
        lastRefresh(),
      ]);
      set((s) => ({
        reviews,
        loaded: true,
        lastError: null,
        // An event may have delivered a newer report while this was in flight.
        lastRefresh: newer(s.lastRefresh, report),
      }));
    } catch (e) {
      set({ loaded: true, lastError: toIpcError(e) });
    }
  },

  reloadList: async () => {
    try {
      set({ reviews: await listReviews(), loaded: true, lastError: null });
    } catch (e) {
      set({ lastError: toIpcError(e) });
    }
  },

  refresh: async () => {
    if (get().refreshing) return;
    set({ refreshing: true });
    try {
      const report = await refreshReviews();
      set((s) => ({ lastRefresh: newer(s.lastRefresh, report) }));
      await get().reloadList();
    } catch (e) {
      set({ lastError: toIpcError(e) });
    } finally {
      set({ refreshing: false });
    }
  },

  markReviewed: async (id) => {
    set({ marking: id });
    try {
      await markReviewed(id);
      await get().reloadList();
    } catch (e) {
      set({ lastError: toIpcError(e) });
    } finally {
      set({ marking: null });
    }
  },

  setTab: (tab) => set({ tab, cursorId: null }),
  select: (id) => set({ selectedId: id, cursorId: id }),

  moveCursor: (delta) => {
    const { reviews, tab, cursorId, selectedId } = get();
    const visible = reviewsInTab(reviews, tab);
    if (visible.length === 0) return;
    const at = visible.findIndex((r) => r.id === (cursorId ?? selectedId));
    const next =
      at < 0
        ? delta > 0
          ? 0
          : visible.length - 1
        : Math.min(visible.length - 1, Math.max(0, at + delta));
    set({ cursorId: visible[next].id });
  },

  openCursor: () => {
    const { cursorId } = get();
    if (cursorId) set({ selectedId: cursorId });
  },

  applyChanged: async (id) => {
    try {
      const record = await getReview(id);
      set((s) => ({
        reviews: s.reviews.map((r) => (r.id === id ? record : r)),
      }));
    } catch {
      // Gone or failing: the `reviews:refreshed` that follows re-reads the
      // whole list anyway.
    }
  },

  applyRefreshed: async (report) => {
    set((s) => ({ lastRefresh: newer(s.lastRefresh, report) }));
    await get().reloadList();
  },
}));

function newer(
  a: RefreshReport | null,
  b: RefreshReport | null,
): RefreshReport | null {
  if (!a) return b;
  if (!b) return a;
  return b.refreshedAt >= a.refreshedAt ? b : a;
}

/** Follow the backend's refresh events; resolves to an unsubscribe. */
export async function subscribeReviewEvents(): Promise<() => void> {
  const offs = await Promise.all([
    onReviewChanged(
      (e) => void useReviewStore.getState().applyChanged(e.reviewId),
    ),
    onReviewsRefreshed((r) => void useReviewStore.getState().applyRefreshed(r)),
  ]);
  return () => offs.forEach((off) => off());
}
