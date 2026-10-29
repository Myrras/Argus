import { describe, expect, it } from "vitest";

import type { FileRequest } from "../ipc/commands";
import { BUSY_RETRY_MS, PREFETCH_BATCH, startPrefetch } from "./prefetch";

const sides = (n: number): FileRequest[][] =>
  Array.from({ length: n }, (_, i) => [
    { sha: "b", path: `f${i}` },
    { sha: "h", path: `f${i}` },
  ]);

function env(opts: { busy?: () => boolean; fail?: boolean } = {}) {
  const idles: (() => void)[] = [];
  const timers: { fn: () => void; ms: number }[] = [];
  const warmed: FileRequest[][] = [];
  let answer: (() => void)[] = [];
  return {
    idles,
    timers,
    warmed,
    /** Answer the warm calls so far, then run what is due. */
    async answer() {
      const run = answer;
      answer = [];
      run.forEach((fn) => fn());
      await Promise.resolve();
      await Promise.resolve();
    },
    runIdle() {
      const run = idles.splice(0);
      run.forEach((fn) => fn());
    },
    env: {
      busy: opts.busy ?? (() => false),
      warm: (requests: FileRequest[]) => {
        warmed.push(requests);
        return new Promise<unknown>((resolve, reject) =>
          answer.push(() =>
            opts.fail ? reject(new Error("offline")) : resolve(requests.length),
          ),
        );
      },
      idle: (fn: () => void) => {
        idles.push(fn);
        return () => {
          const i = idles.indexOf(fn);
          if (i >= 0) idles.splice(i, 1);
        };
      },
      later: (fn: () => void, ms: number) => {
        timers.push({ fn, ms });
        return () => undefined;
      },
    },
  };
}

describe("startPrefetch", () => {
  it("warms the files in order, a batch per idle moment, one at a time", async () => {
    const e = env();
    startPrefetch(sides(PREFETCH_BATCH + 2), e.env);
    expect(e.warmed).toHaveLength(0);
    e.runIdle();
    expect(e.warmed).toHaveLength(1);
    expect(e.warmed[0]).toHaveLength(PREFETCH_BATCH * 2);
    expect(e.warmed[0][0]).toEqual({ sha: "b", path: "f0" });
    // The next batch waits for this one, then for idle time.
    e.runIdle();
    expect(e.warmed).toHaveLength(1);
    await e.answer();
    e.runIdle();
    expect(e.warmed).toHaveLength(2);
    expect(e.warmed[1].map((r) => r.path)).toEqual([
      `f${PREFETCH_BATCH}`,
      `f${PREFETCH_BATCH}`,
      `f${PREFETCH_BATCH + 1}`,
      `f${PREFETCH_BATCH + 1}`,
    ]);
    await e.answer();
    e.runIdle();
    expect(e.warmed).toHaveLength(2);
  });

  it("skips what the page holds and waits while the page reads", () => {
    let busy = true;
    const e = env({ busy: () => busy });
    startPrefetch(sides(2), e.env, (r) => r.path === "f0");
    e.runIdle();
    expect(e.warmed).toHaveLength(0);
    expect(e.timers[0].ms).toBe(BUSY_RETRY_MS);
    busy = false;
    e.timers[0].fn();
    e.runIdle();
    expect(e.warmed[0].map((r) => r.path)).toEqual(["f1", "f1"]);
  });

  it("stops when told, and after a failed batch", async () => {
    const e = env();
    const stop = startPrefetch(sides(40), e.env);
    stop();
    e.runIdle();
    expect(e.warmed).toHaveLength(0);

    const f = env({ fail: true });
    startPrefetch(sides(40), f.env);
    f.runIdle();
    await f.answer();
    f.runIdle();
    expect(f.warmed).toHaveLength(1);
  });
});
