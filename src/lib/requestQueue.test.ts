import { describe, expect, it } from "vitest";

import type { Batched } from "../ipc/types";
import { createRequestQueue } from "./requestQueue";

/** A queue whose calls wait until the test answers them. */
function harness(concurrency = 2, maxBatch = 3) {
  const calls: { group: string; items: string[]; answer: () => void }[] = [];
  let ticks: (() => void)[] = [];
  const q = createRequestQueue<string, string>({
    concurrency,
    maxBatch,
    defer: (fn) => ticks.push(fn),
    send: (group, items) =>
      new Promise<Batched<string>[]>((resolve) =>
        calls.push({
          group,
          items,
          answer: () =>
            resolve(
              items.map((i) =>
                i.startsWith("bad")
                  ? { error: { kind: "notFound", message: i } }
                  : { ok: `text of ${i}` },
              ),
            ),
        }),
      ),
  });
  const flush = () => {
    const run = ticks;
    ticks = [];
    run.forEach((fn) => fn());
  };
  return { q, calls, flush };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("createRequestQueue", () => {
  it("joins the requests of one tick into one call per group", async () => {
    const { q, calls, flush } = harness(4);
    const a = q.request("r1", "a", "a");
    const b = q.request("r1", "b", "b");
    const c = q.request("r2", "c", "c");
    flush();
    // The newest request's group first.
    expect(calls.map((c) => [c.group, c.items])).toEqual([
      ["r2", ["c"]],
      ["r1", ["b", "a"]],
    ]);
    calls.forEach((c) => c.answer());
    expect(await Promise.all([a, b, c])).toEqual([
      "text of a",
      "text of b",
      "text of c",
    ]);
  });

  it("fails only the entry that failed", async () => {
    const { q, calls, flush } = harness();
    const good = q.request("r1", "good", "good");
    const bad = q.request("r1", "bad", "bad");
    flush();
    calls[0].answer();
    await expect(good).resolves.toBe("text of good");
    await expect(bad).rejects.toEqual({ kind: "notFound", message: "bad" });
  });

  it("shares one read for the same key", () => {
    const { q, calls, flush } = harness();
    const one = q.request("r1", "a", "a");
    const two = q.request("r1", "a", "a");
    expect(two).toBe(one);
    flush();
    expect(calls[0].items).toEqual(["a"]);
  });

  it("caps the calls in flight and sends the newest visible first", async () => {
    const { q, calls, flush } = harness(1, 2);
    for (const k of ["a", "b", "c", "d"]) void q.request("r1", k, k);
    flush();
    expect(calls.map((c) => c.items)).toEqual([["d", "c"]]);
    // The user jumped: what they look at now goes next.
    void q.request("r1", "z", "z");
    calls[0].answer();
    await tick();
    expect(calls.map((c) => c.items)).toEqual([
      ["d", "c"],
      ["z", "b"],
    ]);
  });

  it("sends idle reads only when nothing else waits, one call at a time", async () => {
    const { q, calls, flush } = harness(3, 2);
    for (const k of ["i1", "i2", "i3"]) void q.request("r1", k, k, "idle");
    void q.request("r1", "v", "v");
    flush();
    // The visible read alone: idle reads wait for it.
    expect(calls.map((c) => c.items)).toEqual([["v"]]);
    calls[0].answer();
    await tick();
    // Oldest idle first, one call.
    expect(calls.map((c) => c.items)).toEqual([["v"], ["i1", "i2"]]);
  });

  it("moves a queued read up when it is asked for at a higher priority", () => {
    const { q, calls, flush } = harness(1, 1);
    void q.request("r1", "busy", "busy");
    flush();
    void q.request("r1", "later", "later", "idle");
    void q.request("r1", "find", "find", "soon");
    void q.request("r1", "later", "later", "visible");
    calls[0].answer();
    return tick().then(() => {
      expect(calls[1].items).toEqual(["later"]);
    });
  });

  it("rejects every entry when the call fails", async () => {
    const q = createRequestQueue<string, string>({
      concurrency: 1,
      maxBatch: 4,
      send: () => Promise.reject(new Error("offline")),
    });
    await expect(q.request("r1", "a", "a")).rejects.toThrow("offline");
    // Forgotten: a retry reads again.
    await expect(q.request("r1", "a", "a")).rejects.toThrow("offline");
  });
});
