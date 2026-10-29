/**
 * The review page's polite prefetch: once the visible files are in, the
 * rest of a review's files are read into the core's disk cache in idle
 * time, a few at a time (`warm_file_cache`: nothing comes over the IPC), so
 * scrolling on, the find bar, a reopen or the next session read them
 * locally. It never competes with the page: a batch goes only while no
 * contents read waits, one at a time, and it stops when the review closes
 * or a batch fails (offline, rate limited). A review with a local clone
 * needs none: the clone answers at once.
 *
 * Pure over an injected environment (`PrefetchEnv`); `pagePrefetchEnv` is
 * the real one.
 */

import type { FileRequest } from "../ipc/commands";

/**
 * What the prefetch read into the core's cache this session
 * (`reviewId|sha|path`): a review opened again does not warm it twice.
 */
export const warmed = new Set<string>();

/** Files (both sides) per warm call. */
export const PREFETCH_BATCH = 8;
/** How long to wait before looking again while the page reads. */
export const BUSY_RETRY_MS = 500;

export interface PrefetchEnv {
  /** Contents reads are queued or in flight: wait. */
  busy: () => boolean;
  /** Read `requests` into the disk cache. */
  warm: (requests: FileRequest[]) => Promise<unknown>;
  /** Run `fn` when the page is idle; returns a cancel. */
  idle: (fn: () => void) => () => void;
  /** Run `fn` after `ms`; returns a cancel. */
  later: (fn: () => void, ms: number) => () => void;
}

/**
 * Warm `files` (each its `(sha, path)` reads) in order; returns a stop.
 * `inMemory` skips the files the page holds already.
 */
export function startPrefetch(
  files: readonly FileRequest[][],
  env: PrefetchEnv,
  inMemory: (r: FileRequest) => boolean = () => false,
): () => void {
  let stopped = false;
  let cancel: (() => void) | null = null;
  let next = 0;

  const step = () => {
    cancel = null;
    if (stopped) return;
    if (env.busy()) {
      cancel = env.later(() => (cancel = env.idle(step)), BUSY_RETRY_MS);
      return;
    }
    const batch: FileRequest[] = [];
    while (next < files.length && batch.length < PREFETCH_BATCH * 2) {
      batch.push(...files[next++].filter((r) => !inMemory(r)));
    }
    if (batch.length === 0) return;
    env.warm(batch).then(
      () => {
        if (!stopped) cancel = env.idle(step);
      },
      // Offline, rate limited, logged out: leave it to the page's reads.
      () => undefined,
    );
  };

  cancel = env.idle(step);
  return () => {
    stopped = true;
    cancel?.();
  };
}

/** The page's own idle scheduling. */
export function pageIdle(fn: () => void): () => void {
  const w = globalThis as typeof globalThis & {
    requestIdleCallback?: (cb: () => void, o?: { timeout: number }) => number;
    cancelIdleCallback?: (id: number) => void;
  };
  if (w.requestIdleCallback && w.cancelIdleCallback) {
    const id = w.requestIdleCallback(fn, { timeout: 2000 });
    return () => w.cancelIdleCallback!(id);
  }
  const id = setTimeout(fn, 200);
  return () => clearTimeout(id);
}

export function pageLater(fn: () => void, ms: number): () => void {
  const id = setTimeout(fn, ms);
  return () => clearTimeout(id);
}
