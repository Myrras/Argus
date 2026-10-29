/**
 * A review of many files mounts only the diffs near the viewport: the
 * rest are placeholders of their height. A fake IntersectionObserver plays
 * the viewport (which cards are within 1.5 and 3 screens); the editor is
 * stubbed and counts its mounts.
 */

import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));
const editors = vi.hoisted(() => ({ mounts: 0 }));
vi.mock("./DiffEditor", async () => {
  const { useEffect } = await import("react");
  return {
    DiffEditor: (p: { path: string; original: string; modified: string }) => {
      useEffect(() => {
        editors.mounts += 1;
      }, []);
      return (
        <div data-testid={`editor-${p.path}`}>
          {p.original}|{p.modified}
        </div>
      );
    },
  };
});

import { invoke } from "@tauri-apps/api/core";

import type { FileEntry, ReviewFiles } from "../ipc/types";
import {
  clearContentsCache,
  prefetchFile,
  useCodeViewStore,
} from "../state/codeViewStore";
import { clearCommentableCache, useCommentStore } from "../state/commentStore";
import { useFocusStore } from "../state/focusStore";
import { useLayoutStore } from "../state/layoutStore";
import { useMountStore } from "../state/mountStore";
import { useReviewStore } from "../state/reviewStore";
import { useThreadStore } from "../state/threadStore";
import { warmed } from "../lib/prefetch";
import { answerBatches } from "../test/batches";
import { file, review, reviewFiles } from "../test/fixtures";
import { ReviewView } from "./ReviewView";

const mockInvoke = answerBatches(vi.mocked(invoke));
const r = review();
const N = 30;
const paths = Array.from(
  { length: N },
  (_, i) => `src/f${String(i).padStart(2, "0")}.rs`,
);
const files: FileEntry[] = paths.map((path) => file({ path }));
let listed: ReviewFiles;

// -- The viewport -------------------------------------------------------------

/** A card's distance class: within 1.5 screens, within 3, or further. */
type Distance = "close" | "within" | "far";
const observers = new Set<FakeObserver>();
const distance = new Map<string, Distance>();

class FakeObserver {
  readonly els = new Set<Element>();
  constructor(
    readonly cb: IntersectionObserverCallback,
    readonly opts: IntersectionObserverInit = {},
  ) {
    observers.add(this);
  }
  observe(el: Element) {
    this.els.add(el);
    this.report(el);
  }
  unobserve(el: Element) {
    this.els.delete(el);
  }
  disconnect() {
    observers.delete(this);
  }
  takeRecords() {
    return [];
  }
  /** Whether `el` intersects this observer's margin. */
  report(el: Element) {
    const d = distance.get((el as HTMLElement).dataset.path ?? "") ?? "far";
    const wide = this.opts.rootMargin?.startsWith("300%");
    const hit = d === "close" || (wide && d === "within");
    this.cb(
      [{ isIntersecting: hit, target: el } as IntersectionObserverEntry],
      this as unknown as IntersectionObserver,
    );
  }
}

/** Move the viewport: `near` cards close, `within` ones in the margin. */
function view(near: string[], within: string[] = []) {
  distance.clear();
  for (const p of within) distance.set(p, "within");
  for (const p of near) distance.set(p, "close");
  act(() => {
    for (const o of [...observers]) for (const el of o.els) o.report(el);
  });
}

// -- Backend --------------------------------------------------------------------

function backend(cmd: string, args?: unknown) {
  const a = (args ?? {}) as Record<string, unknown>;
  switch (cmd) {
    case "list_threads":
      return Promise.resolve([]);
    case "list_files":
      return Promise.resolve(listed);
    case "file_contents":
      return Promise.resolve(`${String(a.path)}@${String(a.sha).slice(0, 1)}`);
    case "commentable_lines":
      return Promise.resolve({
        reviewId: r.id,
        path: a.path,
        baseSha: listed.reviewBaseSha,
        headSha: listed.headSha,
        hunks: [{ old: [1], new: [1] }],
        outside: "none",
      });
    default:
      return Promise.resolve(null);
  }
}

const mounted = () =>
  paths.filter((p) => screen.queryByTestId(`editor-${p}`) !== null);
const placeholders = () =>
  document.querySelectorAll('[data-slot="unmounted-diff"]').length;

beforeEach(() => {
  vi.stubGlobal("IntersectionObserver", FakeObserver);
  observers.clear();
  distance.clear();
  editors.mounts = 0;
  warmed.clear();
  localStorage.clear();
  clearContentsCache();
  clearCommentableCache();
  useMountStore.setState({ pins: {} });
  useFocusStore.setState({ reviewId: null, item: null });
  useCommentStore.setState({
    drafts: {},
    composer: null,
    replies: {},
    posting: {},
    focus: null,
  });
  listed = reviewFiles({ reviewId: r.id, files });
  mockInvoke.mockImplementation(backend as typeof invoke);
  useCodeViewStore.setState({ byReview: {} });
  useThreadStore.setState({ byReview: {} });
  useReviewStore.setState({ reviews: [r], selectedId: r.id });
  useLayoutStore.setState({
    diffLayout: "unified",
    fileListMode: "flat",
    fileTreeOpen: true,
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

async function open() {
  render(<ReviewView review={r} />);
  await screen.findByText(`0 of ${N} files viewed`);
}

describe("a review of many files", () => {
  it("mounts only the diffs near the viewport; the rest hold their place", async () => {
    await open();
    expect(placeholders()).toBe(N);
    view(paths.slice(0, 3));
    await waitFor(() => expect(mounted()).toEqual(paths.slice(0, 3)));
    expect(placeholders()).toBe(N - 3);
    // A placeholder has the estimated height of its diff.
    const far = document.querySelector<HTMLElement>(
      `[data-path="${paths[20]}"] [data-slot="unmounted-diff"]`,
    )!;
    expect(parseInt(far.style.height, 10)).toBeGreaterThan(0);
  });

  it("keeps a diff while it is within reach, drops it once well away", async () => {
    await open();
    view([paths[0]]);
    await screen.findByTestId(`editor-${paths[0]}`);
    // Scrolled on a bit: still within three screens.
    view([paths[5]], [paths[0]]);
    await screen.findByTestId(`editor-${paths[5]}`);
    expect(screen.getByTestId(`editor-${paths[0]}`)).toBeInTheDocument();
    // Far away now.
    view([paths[20]]);
    await waitFor(() =>
      expect(screen.queryByTestId(`editor-${paths[0]}`)).toBeNull(),
    );
    expect(mounted()).toEqual([paths[20]]);
  });

  it("mounts the selected file, and a far one a reveal asks for", async () => {
    await open();
    act(() => useCodeViewStore.getState().select(r.id, paths[12]));
    await screen.findByTestId(`editor-${paths[12]}`);
    // n/p or the find bar going to a thread in a file far down.
    act(() => void prefetchFile(r.id, paths[25]));
    await screen.findByTestId(`editor-${paths[25]}`);
    // Once it came near, nearness keeps it, not the pin.
    view([paths[25]]);
    await waitFor(() => expect(useMountStore.getState().pins).toEqual({}));
    expect(screen.getByTestId(`editor-${paths[25]}`)).toBeInTheDocument();
  });

  it("keeps the file with the open composer mounted", async () => {
    await open();
    view([paths[2]]);
    await screen.findByTestId(`editor-${paths[2]}`);
    act(() =>
      useCommentStore.getState().openComposer({
        reviewId: r.id,
        path: paths[2],
        side: "new",
        anchor: 1,
        startLine: null,
        line: 1,
        headSha: listed.headSha,
        baseSha: listed.baseSha,
        outside: null,
      }),
    );
    view([paths[28]]);
    await screen.findByTestId(`editor-${paths[28]}`);
    expect(screen.getByTestId(`editor-${paths[2]}`)).toBeInTheDocument();
  });

  it("remembers what the user did in a card across an unmount", async () => {
    listed = reviewFiles({
      reviewId: r.id,
      files: [
        file({ path: paths[0], additions: 2000, deletions: 0 }),
        ...files.slice(1),
      ],
    });
    await open();
    view([paths[0]]);
    fireEvent.click(await screen.findByRole("button", { name: "Load anyway" }));
    await screen.findByTestId(`editor-${paths[0]}`);
    view([paths[29]]);
    await waitFor(() =>
      expect(screen.queryByTestId(`editor-${paths[0]}`)).toBeNull(),
    );
    view([paths[0]]);
    // Still loaded: no second "Load anyway".
    await screen.findByTestId(`editor-${paths[0]}`);
    expect(screen.queryByRole("button", { name: "Load anyway" })).toBeNull();
  });

  it("reads the visible files first, in one call, then the rest in idle time", async () => {
    await open();
    view(paths.slice(0, 2));
    await waitFor(() => expect(mounted()).toEqual(paths.slice(0, 2)));
    const batches = mockInvoke.mock.calls.filter(
      ([cmd]) => cmd === "file_contents_batch",
    );
    expect(batches).toHaveLength(1);
    const asked = (batches[0][1] as { requests: { path: string }[] }).requests;
    expect(new Set(asked.map((q) => q.path))).toEqual(
      new Set(paths.slice(0, 2)),
    );
    // The idle prefetch warms the core's cache with the others, never
    // the ones the page holds.
    await waitFor(
      () =>
        expect(
          mockInvoke.mock.calls.some(([cmd]) => cmd === "warm_file_cache"),
        ).toBe(true),
      { timeout: 3000 },
    );
    const warmed = mockInvoke.mock.calls
      .filter(([cmd]) => cmd === "warm_file_cache")
      .flatMap(([, a]) => (a as { requests: { path: string }[] }).requests)
      .map((q) => q.path);
    expect(warmed).not.toContain(paths[0]);
    expect(warmed).toContain(paths[2]);
  });
});
