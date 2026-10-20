/**
 * Review list state: the active reviews, the selected tab and review, and
 * the last refresh. Mirrors the backend — every action calls a typed IPC
 * wrapper, then re-reads; the refresh events (`subscribeReviewEvents`) keep
 * it current between user actions.
 *
 * A failed call (loading the list, a refresh, submitting a review) is an
 * error toast, the family's way. Refresh errors are not: they live in
 * `lastRefresh.errors`, per provider, show in the sidebar's Providers
 * section and never block the list.
 */

import { create } from "zustand";

import {
  getReview,
  lastRefresh,
  listReviews,
  refreshReviews,
  submitReview,
  toIpcError,
} from "../ipc/commands";
import { onReviewChanged, onReviewsRefreshed } from "../ipc/events";
import type {
  IpcError,
  RefreshReport,
  ReviewId,
  ReviewRecord,
  ReviewVerdict,
} from "../ipc/types";
import { latestOnly } from "../lib/latest";
import {
  VERDICT_DONE,
  reviewRef,
  reviewsInTab,
  type ReviewTab,
} from "../lib/reviews";
import { toastError, toastSuccess } from "./toastStore";

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
  /** The review whose review is being submitted (approve included). */
  approving: ReviewId | null;
  /**
   * Reviews just approved from Argus, with the tab they were in: they stay
   * there as "Approved ✓" for `APPROVED_LINGER_MS`, then leave (unless a
   * refresh shows the approval gone, which brings them back as usual).
   */
  justApproved: Partial<Record<ReviewId, ReviewTab>>;
  lastRefresh: RefreshReport | null;
  /** Initial load: the list plus the last refresh the backend saw. */
  load: () => Promise<void>;
  reloadList: () => Promise<void>;
  /** Refresh every provider now (the button and `r`). */
  refresh: () => Promise<void>;
  /**
   * Submit the user's review on the platform: `verdict` at `headSha` (the
   * head the dialog named), with an optional summary. Resolves to the
   * error, or null once submitted. An approval leaves the active lists
   * after `APPROVED_LINGER_MS`. While another submit runs: a `busy` error.
   */
  submit: (
    id: ReviewId,
    headSha: string,
    verdict: ReviewVerdict,
    summary?: string | null,
  ) => Promise<IpcError | null>;
  /** `submit` with "approve" and no summary. */
  approve: (id: ReviewId, headSha: string) => Promise<IpcError | null>;
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

/**
 * Reads can answer out of order: only the newest read of the list (`LIST`)
 * or of one review applies its answer. A list read keeps the records of the
 * reviews re-read one by one since it began: theirs are newer.
 */
const reads = latestOnly<string>();
const LIST = "\u0000list";

function withNewer(
  listed: ReviewRecord[],
  current: ReviewRecord[],
  moved: (id: ReviewId) => boolean,
): ReviewRecord[] {
  const merged = listed.map((r) =>
    moved(r.id) ? (current.find((c) => c.id === r.id) ?? r) : r,
  );
  // The listed records this answer applies were read after any per-review
  // read still in flight from before the list began: number those ids so
  // the older answers drop instead of overwriting these. (Ids re-read since
  // the list began keep theirs.) After the merge: `moved` would see this.
  for (const r of listed) if (!moved(r.id)) reads.begin(r.id);
  return merged;
}

/**
 * `listed` plus the reviews still lingering as approved (the list leaves
 * approved ones out), each at the place it had in `current`.
 */
function withLingering(
  listed: ReviewRecord[],
  current: ReviewRecord[],
  lingering: Partial<Record<ReviewId, ReviewTab>>,
): ReviewRecord[] {
  const out = [...listed];
  const ids = new Set(listed.map((r) => r.id));
  current.forEach((r, i) => {
    if (lingering[r.id] && !ids.has(r.id)) {
      out.splice(Math.min(i, out.length), 0, r);
    }
  });
  return out;
}

export const useReviewStore = create<ReviewStoreState>((set, get) => ({
  reviews: [],
  loaded: false,
  tab: "needsReview",
  selectedId: null,
  cursorId: null,
  refreshing: false,
  approving: null,
  justApproved: {},
  lastRefresh: null,

  load: async () => {
    const current = reads.begin(LIST);
    const moved = reads.mark();
    try {
      const [reviews, report] = await Promise.all([
        listReviews(),
        lastRefresh(),
      ]);
      if (!current()) {
        set((s) => ({
          loaded: true,
          lastRefresh: newer(s.lastRefresh, report),
        }));
        return;
      }
      const merged = withLingering(
        withNewer(reviews, get().reviews, moved),
        get().reviews,
        get().justApproved,
      );
      set((s) => ({
        reviews: merged,
        loaded: true,
        // An event may have delivered a newer report while this was in flight.
        lastRefresh: newer(s.lastRefresh, report),
      }));
    } catch (e) {
      set({ loaded: true });
      toastError("Could not load the reviews", toIpcError(e).message);
    }
  },

  reloadList: async () => {
    const current = reads.begin(LIST);
    const moved = reads.mark();
    try {
      const reviews = await listReviews();
      if (current()) {
        set({
          reviews: withLingering(
            withNewer(reviews, get().reviews, moved),
            get().reviews,
            get().justApproved,
          ),
          loaded: true,
        });
      }
    } catch (e) {
      toastError("Could not load the reviews", toIpcError(e).message);
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
      toastError("Could not refresh", toIpcError(e).message);
    } finally {
      set({ refreshing: false });
    }
  },

  submit: async (id, headSha, verdict, summary = null) => {
    if (get().approving) {
      return {
        kind: "busy",
        message: "Another review is still being submitted.",
      };
    }
    const before = get().reviews.find((r) => r.id === id);
    set({ approving: id });
    try {
      const record = await submitReview(
        id,
        headSha,
        verdict,
        summary?.trim() ? summary : null,
      );
      toastSuccess(
        `${VERDICT_DONE[verdict]} ${reviewRef(record)} at ${headSha.slice(0, 7)}`,
      );
      set((s) => ({
        reviews: s.reviews.map((r) => (r.id === id ? record : r)),
      }));
      if (verdict === "approve" && before && before.status !== "approved") {
        lingerApproved(id, before.status);
      }
      // Done with it: the middle column leaves the review (the list's
      // cursor stays on it while "Approved ✓" lingers).
      if (get().selectedId === id) set({ selectedId: null });
      await get().reloadList();
      return null;
    } catch (e) {
      const err = toIpcError(e);
      toastError("Could not submit the review", err.message);
      // A moved head: show what the backend has now.
      if (err.kind === "headMoved") await get().reloadList();
      return err;
    } finally {
      set({ approving: null });
    }
  },

  approve: (id, headSha) => get().submit(id, headSha, "approve"),

  setTab: (tab) => set({ tab, cursorId: null }),
  select: (id) => set({ selectedId: id, cursorId: id }),

  moveCursor: (delta) => {
    const { reviews, tab, cursorId, selectedId, justApproved } = get();
    const visible = reviewsInTab(reviews, tab, justApproved);
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
    const current = reads.begin(id);
    try {
      const record = await getReview(id);
      if (!current()) return;
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

/** How long an approved review stays in its tab as "Approved ✓". */
export const APPROVED_LINGER_MS = 3000;

/** Keep `id` in `tab` as approved for a moment, then let it go. */
function lingerApproved(id: ReviewId, tab: ReviewTab) {
  useReviewStore.setState((s) => ({
    justApproved: { ...s.justApproved, [id]: tab },
  }));
  setTimeout(() => {
    useReviewStore.setState((s) => {
      if (!(id in s.justApproved)) return {};
      const next = { ...s.justApproved };
      delete next[id];
      return { justApproved: next };
    });
  }, APPROVED_LINGER_MS);
}

/**
 * The later of two reports by time. Compared as instants, not strings: the
 * core leaves out a zero fraction, so `…:01Z` would sort after the later
 * `…:01.2Z`. A tie (or an unreadable time) goes to `b`, the one that came in
 * last.
 */
function newer(
  a: RefreshReport | null,
  b: RefreshReport | null,
): RefreshReport | null {
  if (!a) return b;
  if (!b) return a;
  return Date.parse(a.refreshedAt) > Date.parse(b.refreshedAt) ? a : b;
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
