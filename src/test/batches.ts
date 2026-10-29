/**
 * The batch reads (`file_contents_batch`, `commentable_lines_batch`) as the
 * core answers them: each entry read as its single command, through the
 * (mocked) `invoke`, so a test's own `file_contents` / `commentable_lines`
 * answers and its call assertions keep working. Wrap a test's invoke
 * implementation: `mockInvoke.mockImplementation(withBatches(impl))`.
 */

import { invoke } from "@tauri-apps/api/core";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Impl = (cmd: string, args?: any) => any;

const settle = (p: unknown) =>
  Promise.resolve(p).then(
    (ok) => ({ ok }),
    (error: unknown) => ({ error }),
  );

export function withBatches<F extends Impl>(impl: F): F {
  return ((cmd: string, args?: Record<string, unknown>) => {
    if (cmd === "file_contents_batch") {
      const requests = args?.requests as { sha: string; path: string }[];
      return Promise.all(
        requests.map((r) =>
          settle(
            invoke("file_contents", {
              reviewId: args?.reviewId,
              sha: r.sha,
              path: r.path,
            }),
          ),
        ),
      );
    }
    if (cmd === "commentable_lines_batch") {
      const paths = args?.paths as string[];
      return Promise.all(
        paths.map((path) =>
          settle(
            invoke("commentable_lines", { reviewId: args?.reviewId, path }),
          ),
        ),
      );
    }
    return impl(cmd, args);
  }) as F;
}

interface ImplMock {
  mockImplementation: (fn: never) => unknown;
  mockImplementationOnce: (fn: never) => unknown;
}

/**
 * Make every implementation `mock` (the mocked `invoke`) is given answer
 * the batch commands too ([`withBatches`]), so a test file opts in once:
 * `const mockInvoke = answerBatches(vi.mocked(invoke));`.
 */
export function answerBatches<M extends ImplMock>(mock: M): M {
  const impl = mock.mockImplementation.bind(mock) as (fn: Impl) => unknown;
  const once = mock.mockImplementationOnce.bind(mock) as (fn: Impl) => unknown;
  Object.assign(mock, {
    mockImplementation: (fn: Impl) => impl(withBatches(fn)),
    mockImplementationOnce: (fn: Impl) => once(withBatches(fn)),
  });
  return mock;
}
