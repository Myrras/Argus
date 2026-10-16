/**
 * The review view's state per review: the changed files (and the viewed
 * marks, which the backend owns), the "since my last review" mode, the
 * selected stop (the Conversation section, `CONVERSATION`, or a file's path)
 * and the visible order j/k walk. File contents are cached in memory per
 * (review, sha, path) outside the store: they are large and never rendered
 * from state.
 */

import { create } from "zustand";

import {
  fileContents,
  listFiles,
  setFileViewed,
  toIpcError,
} from "../ipc/commands";
import { onReviewChanged } from "../ipc/events";
import type { IpcError, ReviewFiles, ReviewId } from "../ipc/types";
import { latestOnly } from "../lib/latest";

/**
 * The Conversation section's key in `selected` / `order`: the first stop of
 * j/k. A NUL can never be part of a file path, so it cannot collide.
 */
export const CONVERSATION = "\u0000conversation";

export interface ReviewCode {
  /** The mode `files` was loaded in (and the next load uses). */
  since: boolean;
  files: ReviewFiles | null;
  loading: boolean;
  error: IpcError | null;
  /** The selected file's path, or `CONVERSATION`. */
  selected: string | null;
  /** `CONVERSATION`, then the files as shown (filtered, tree order). */
  order: string[];
}

interface CodeViewState {
  byReview: Record<ReviewId, ReviewCode>;
  /** Load (or reload) the review's files in `since` mode (default: the current one). */
  load: (id: ReviewId, since?: boolean) => Promise<void>;
  setViewed: (id: ReviewId, path: string, viewed: boolean) => Promise<void>;
  select: (id: ReviewId, path: string | null) => void;
  setOrder: (id: ReviewId, order: string[]) => void;
  /** Select the next (+1) or previous (-1) stop of the shown order. */
  move: (id: ReviewId, delta: 1 | -1) => string | null;
  /** Flip the selected file's viewed mark (nothing on Conversation). */
  toggleSelectedViewed: (id: ReviewId) => Promise<void>;
  /** `review:changed`: reload when that review's files are loaded. */
  applyChanged: (id: ReviewId) => Promise<void>;
}

const EMPTY: ReviewCode = {
  since: false,
  files: null,
  loading: false,
  error: null,
  selected: null,
  order: [],
};

const reads = latestOnly<ReviewId>();

export const useCodeViewStore = create<CodeViewState>((set, get) => {
  const patch = (id: ReviewId, p: Partial<ReviewCode>) =>
    set((s) => ({
      byReview: {
        ...s.byReview,
        [id]: { ...(s.byReview[id] ?? EMPTY), ...p },
      },
    }));
  const entry = (id: ReviewId) => get().byReview[id] ?? EMPTY;

  return {
    byReview: {},

    load: async (id, since) => {
      const mode = since ?? entry(id).since;
      // A newer read (another mode, or a re-read at a newer head) wins.
      const current = reads.begin(id);
      patch(id, { since: mode, loading: true });
      try {
        const files = await listFiles(id, mode);
        if (!current()) return;
        patch(id, { files, loading: false, error: null });
      } catch (e) {
        if (!current()) return;
        patch(id, { loading: false, error: toIpcError(e) });
      }
    },

    setViewed: async (id, path, viewed) => {
      const before = entry(id).files;
      if (!before) return;
      const apply = (paths: Set<string> | ((p: string) => boolean)) => {
        const has =
          typeof paths === "function" ? paths : (p: string) => paths.has(p);
        const cur = entry(id).files;
        if (!cur) return;
        patch(id, {
          files: {
            ...cur,
            files: cur.files.map((f) => ({ ...f, viewed: has(f.path) })),
          },
        });
      };
      // Optimistic, then whatever the backend says is viewed.
      const prior = new Set(
        before.files.filter((f) => f.viewed).map((f) => f.path),
      );
      apply((p) => (p === path ? viewed : prior.has(p)));
      try {
        apply(new Set(await setFileViewed(id, path, viewed)));
      } catch (e) {
        apply(prior);
        patch(id, { error: toIpcError(e) });
      }
    },

    select: (id, path) => patch(id, { selected: path }),

    setOrder: (id, order) => {
      const cur = entry(id);
      if (
        cur.order.length === order.length &&
        cur.order.every((p, i) => p === order[i])
      )
        return;
      patch(id, { order });
    },

    move: (id, delta) => {
      const { order, selected } = entry(id);
      if (order.length === 0) return null;
      const at = selected ? order.indexOf(selected) : -1;
      const next =
        at < 0
          ? delta > 0
            ? 0
            : order.length - 1
          : Math.min(order.length - 1, Math.max(0, at + delta));
      const path = order[next];
      patch(id, { selected: path });
      return path;
    },

    toggleSelectedViewed: async (id) => {
      const { selected, files } = entry(id);
      const file = files?.files.find((f) => f.path === selected);
      if (file) await get().setViewed(id, file.path, !file.viewed);
    },

    applyChanged: async (id) => {
      if (get().byReview[id]?.files) await get().load(id);
    },
  };
});

/** Follow `review:changed`; resolves to an unsubscribe. */
export function subscribeCodeViewEvents(): Promise<() => void> {
  return onReviewChanged(
    (e) => void useCodeViewStore.getState().applyChanged(e.reviewId),
  );
}

// -- File contents --------------------------------------------------------

const contents = new Map<string, Promise<string>>();

/**
 * The content of `path` at `sha`, fetched once per (review, sha, path) and
 * kept in memory. A failed fetch is forgotten so a retry fetches again.
 */
export function loadContents(
  id: ReviewId,
  sha: string,
  path: string,
): Promise<string> {
  const key = `${id}|${sha}|${path}`;
  let p = contents.get(key);
  if (!p) {
    p = fileContents(id, sha, path);
    contents.set(key, p);
    p.catch(() => contents.delete(key));
  }
  return p;
}

/** Tests: forget every cached file. */
export function clearContentsCache() {
  contents.clear();
}
