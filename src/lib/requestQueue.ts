/**
 * Per-file reads (contents, commentable lines) as batched, prioritised IPC
 * calls. A review page asks for many files at once (every card near the
 * viewport, the find bar, the idle prefetch); one call per file would cost
 * a round trip each and run in whatever order the cards mounted.
 *
 * - **Batching**: requests made in the same tick join one call per group
 *   (a review), up to `maxBatch` items.
 * - **Priority**: `visible` (a card near the viewport, the selected file, a
 *   reveal) goes before `soon` (the find bar) before `idle` (prefetch).
 *   Among `visible` requests the newest goes first: after a jump, what the
 *   user looks at now beats what they scrolled past. Asking again for a
 *   queued item at a higher priority moves it up.
 * - **Concurrency**: at most `concurrency` calls at once, and an `idle`
 *   call only when nothing else waits, one at a time.
 * - The same key asked twice while queued or in flight shares one read.
 *
 * Pure over the injected `send`, so the rules test without IPC.
 */

import type { Batched, IpcError } from "../ipc/types";

export type Priority = "visible" | "soon" | "idle";

const RANK: Record<Priority, number> = { visible: 2, soon: 1, idle: 0 };

interface Job<I, T> {
  group: string;
  key: string;
  item: I;
  rank: number;
  seq: number;
  resolve: (v: T) => void;
  reject: (e: unknown) => void;
  promise: Promise<T>;
}

export interface RequestQueue<I, T> {
  /** Read `item` (deduped by `key`) in `group`'s next call. */
  request(group: string, key: string, item: I, priority?: Priority): Promise<T>;
  /** Queued (not yet sent) requests, for tests and the prefetch. */
  queued(): number;
  /** In-flight calls. */
  active(): number;
  /** Drop every queued request (they stay unanswered): tests. */
  reset(): void;
}

export function createRequestQueue<I, T>(opts: {
  send: (group: string, items: I[]) => Promise<Batched<T>[]>;
  maxBatch: number;
  concurrency: number;
  /** Run `fn` after the current tick (tests: synchronous flushes). */
  defer?: (fn: () => void) => void;
}): RequestQueue<I, T> {
  const defer = opts.defer ?? ((fn) => queueMicrotask(fn));
  let queue: Job<I, T>[] = [];
  const byKey = new Map<string, Job<I, T>>();
  let seq = 0;
  let running = 0;
  let idleRunning = 0;
  let scheduled = false;

  const schedule = () => {
    if (scheduled) return;
    scheduled = true;
    defer(() => {
      scheduled = false;
      pump();
    });
  };

  /** The job to send next: highest rank; newest first unless idle. */
  const best = (): Job<I, T> | undefined => {
    let pick: Job<I, T> | undefined;
    for (const j of queue) {
      if (
        !pick ||
        j.rank > pick.rank ||
        (j.rank === pick.rank &&
          (j.rank === RANK.idle ? j.seq < pick.seq : j.seq > pick.seq))
      ) {
        pick = j;
      }
    }
    return pick;
  };

  const pump = () => {
    while (running < opts.concurrency && queue.length > 0) {
      const first = best()!;
      const idle = first.rank === RANK.idle;
      if (idle && (idleRunning > 0 || running > 0)) return;
      // The batch: the best job's group, best first.
      const batch = queue
        .filter((j) => j.group === first.group && (idle || j.rank > RANK.idle))
        .sort((a, b) =>
          b.rank !== a.rank
            ? b.rank - a.rank
            : a.rank === RANK.idle
              ? a.seq - b.seq
              : b.seq - a.seq,
        )
        .slice(0, opts.maxBatch);
      const sent = new Set(batch);
      queue = queue.filter((j) => !sent.has(j));
      running++;
      if (idle) idleRunning++;
      const done = () => {
        running--;
        if (idle) idleRunning--;
        for (const j of batch) {
          if (byKey.get(j.key) === j) byKey.delete(j.key);
        }
        pump();
      };
      opts
        .send(
          first.group,
          batch.map((j) => j.item),
        )
        .then(
          (answers) => {
            batch.forEach((j, i) => {
              const a = answers?.[i];
              if (a && "ok" in a) j.resolve(a.ok);
              else if (a && "error" in a) j.reject(a.error);
              else j.reject(missing());
            });
            done();
          },
          (e: unknown) => {
            for (const j of batch) j.reject(e);
            done();
          },
        );
    }
  };

  return {
    request(group, key, item, priority = "visible") {
      const rank = RANK[priority];
      const known = byKey.get(key);
      if (known) {
        if (rank > known.rank && queue.includes(known)) {
          known.rank = rank;
          known.seq = ++seq;
          schedule();
        }
        return known.promise;
      }
      let resolve!: (v: T) => void;
      let reject!: (e: unknown) => void;
      const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
      });
      const job: Job<I, T> = {
        group,
        key,
        item,
        rank,
        seq: ++seq,
        resolve,
        reject,
        promise,
      };
      queue.push(job);
      byKey.set(key, job);
      schedule();
      return promise;
    },
    queued: () => queue.length,
    active: () => running,
    reset() {
      queue = [];
      byKey.clear();
      running = 0;
      idleRunning = 0;
    },
  };
}

function missing(): IpcError {
  return { kind: "internal", message: "No answer for this file." };
}
