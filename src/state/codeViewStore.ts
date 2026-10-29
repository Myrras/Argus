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
  fileContentsBatch,
  listFiles,
  setFileViewed,
  toIpcError,
  type FileRequest,
} from "../ipc/commands";
import { onReviewChanged } from "../ipc/events";
import type { FileEntry, IpcError, ReviewFiles, ReviewId } from "../ipc/types";
import { latestOnly } from "../lib/latest";
import { createRequestQueue, type Priority } from "../lib/requestQueue";
import { pinCard } from "./mountStore";

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
  /**
   * `selected` was set without the view's scroll to the card (the find
   * bar scrolls to its match itself).
   */
  quietly?: string | null;
}

interface CodeViewState {
  byReview: Record<ReviewId, ReviewCode>;
  /** Load (or reload) the review's files in `since` mode (default: the current one). */
  load: (id: ReviewId, since?: boolean) => Promise<void>;
  setViewed: (id: ReviewId, path: string, viewed: boolean) => Promise<void>;
  /** `scroll: false`: the view leaves the page where it is. */
  select: (
    id: ReviewId,
    path: string | null,
    opts?: { scroll?: boolean },
  ) => void;
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
  quietly: null,
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

    select: (id, path, opts) =>
      patch(id, {
        selected: path,
        quietly: opts?.scroll === false ? path : null,
      }),

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
      patch(id, { selected: path, quietly: null });
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

/**
 * Contents kept in memory, most recently used last, up to `MEMORY_CHARS`
 * in all: the hot layer over the core's disk cache. Large and never
 * rendered from state, so outside the store.
 */
const memory = new Map<string, string>();
let memoryChars = 0;
/** About 128 MB of UTF-16. */
export const MEMORY_CHARS = 64 * 1024 * 1024;

function remember(key: string, text: string) {
  const old = memory.get(key);
  if (old !== undefined) memoryChars -= old.length;
  memory.delete(key);
  memory.set(key, text);
  memoryChars += text.length;
  for (const [k, v] of memory) {
    if (memoryChars <= MEMORY_CHARS || k === key) break;
    memory.delete(k);
    memoryChars -= v.length;
  }
}

/** Contents reads: batched per review, the visible files first. */
const contentReads = createRequestQueue<FileRequest, string>({
  send: (reviewId, requests) => fileContentsBatch(reviewId, requests),
  // A card asks for both sides: 12 cards' worth per call.
  maxBatch: 24,
  concurrency: 3,
});

/**
 * The content of `path` at `sha`, fetched once per (review, sha, path) and
 * kept in memory (bounded, least recently used out). A failed fetch is
 * forgotten so a retry fetches again. `priority` orders it among the other
 * reads waiting (`lib/requestQueue`).
 */
export function loadContents(
  id: ReviewId,
  sha: string,
  path: string,
  priority: Priority = "visible",
): Promise<string> {
  const key = `${id}|${sha}|${path}`;
  const kept = memory.get(key);
  if (kept !== undefined) {
    remember(key, kept);
    return Promise.resolve(kept);
  }
  const p = contentReads.request(id, key, { sha, path }, priority);
  p.then(
    (text) => remember(key, text),
    () => undefined,
  );
  return p;
}

/** Whether contents reads are queued or in flight (the prefetch waits). */
export function contentsBusy(): boolean {
  return contentReads.queued() > 0 || contentReads.active() > 0;
}

/** Whether `path` at `sha` is in memory now (no read needed). */
export function hasContents(id: ReviewId, sha: string, path: string): boolean {
  return memory.has(`${id}|${sha}|${path}`);
}

/** Above this many changed lines a diff waits for "Load anyway". */
export const LARGE_DIFF_LINES = 1500;

/** No editor shows this file until "Load anyway" (or ever: binary). */
export function waitsForLoadAnyway(f: FileEntry): boolean {
  return f.tooLarge || f.additions + f.deletions > LARGE_DIFF_LINES;
}

/** A file's base and head text for its diff ("" for the missing side). */
export function loadFileSides(
  review: ReviewFiles,
  f: FileEntry,
  priority: Priority = "visible",
): Promise<[string, string]> {
  const base =
    f.status === "added"
      ? Promise.resolve("")
      : loadContents(
          review.reviewId,
          review.baseSha,
          f.oldPath ?? f.path,
          priority,
        );
  const head =
    f.status === "removed"
      ? Promise.resolve("")
      : loadContents(review.reviewId, review.headSha, f.path, priority);
  return Promise.all([base, head]);
}

/** The `(sha, path)` reads a file's diff needs. */
export function fileSides(review: ReviewFiles, f: FileEntry): FileRequest[] {
  const out: FileRequest[] = [];
  if (f.status !== "added") {
    out.push({ sha: review.baseSha, path: f.oldPath ?? f.path });
  }
  if (f.status !== "removed") out.push({ sha: review.headSha, path: f.path });
  return out;
}

/**
 * Start (or join) the load of a shown file's two sides, as its card would,
 * and mount its card's diff: a keyboard move to one of its threads (or the
 * find bar going to a match in it) waits for it. Null when no editor
 * would show the file (unknown, binary, large) — nothing to wait for.
 */
export function prefetchFile(
  id: ReviewId,
  path: string,
): Promise<[string, string]> | null {
  // A card far from the viewport has no diff mounted: pin it, so the
  // reveal finds its editor (`mountStore`).
  pinCard(id, path);
  const files = useCodeViewStore.getState().byReview[id]?.files;
  const f = files?.files.find((x) => x.path === path);
  if (!files || !f || f.binary || waitsForLoadAnyway(f)) return null;
  return loadFileSides(files, f);
}

/** Tests: forget every cached file and queued read. */
export function clearContentsCache() {
  memory.clear();
  memoryChars = 0;
  contentReads.reset();
}
