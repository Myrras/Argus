import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";

import type { StoredThread, ThreadId } from "../ipc/types";
import { useHistoryStore } from "../state/historyStore";
import { useThreadStore } from "../state/threadStore";
import { useToastStore } from "../state/toastStore";
import { thread } from "../test/fixtures";
import { ListedThread, ThreadBlock } from "./ThreadBlock";

const mockInvoke = vi.mocked(invoke);
const NOW = Date.parse("2026-10-16T12:00:00Z");
const t = thread({ resolvable: true, author: "bob-fixture" });

/** The thread as the store has it, the way the review view passes it on. */
function Live({ id }: { id: ThreadId }) {
  const current = useThreadStore((s) =>
    s.byReview[t.reviewId]?.threads.find((x) => x.threadId === id),
  );
  return current ? (
    <ThreadBlock thread={current} provider="gitlab" now={NOW} />
  ) : null;
}

function seed(threads: StoredThread[]) {
  useThreadStore.setState({
    byReview: { [t.reviewId]: { threads, loading: false, error: null } },
    resolving: {},
  });
}

const resolveCalls = () =>
  mockInvoke.mock.calls.filter(([cmd]) => cmd === "resolve_thread");

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockResolvedValue([]);
  useToastStore.setState({ toasts: [] });
  seed([t]);
});

describe("resolving from the thread", () => {
  it("offers Resolve or Unresolve only where the platform can", () => {
    const talk = thread({ resolvable: false, anchor: null });
    const done = thread({ resolvable: true, resolved: true });
    seed([t, talk, done]);
    render(
      <>
        <Live id={t.threadId} />
        <Live id={talk.threadId} />
        <Live id={done.threadId} />
      </>,
    );
    expect(screen.getAllByRole("button", { name: /^Resolve / })).toHaveLength(
      1,
    );
    expect(screen.getAllByRole("button", { name: /^Unresolve / })).toHaveLength(
      1,
    );
    expect(screen.getAllByText("Resolved")).toHaveLength(1);
    // Showing it writes nothing.
    expect(resolveCalls()).toHaveLength(0);
  });

  it("writes once on the click, is busy meanwhile, then shows the re-read", async () => {
    let answer: (v: unknown) => void = () => {};
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "resolve_thread") return new Promise((r) => (answer = r));
      if (cmd === "list_threads") {
        return Promise.resolve([{ ...t, resolved: true }]);
      }
      return Promise.resolve([]);
    });
    render(<Live id={t.threadId} />);
    fireEvent.click(
      screen.getByRole("button", { name: "Resolve bob-fixture's thread" }),
    );
    const busy = screen.getByRole("button", {
      name: "Resolve bob-fixture's thread",
    });
    expect(busy).toBeDisabled();
    expect(busy).toHaveTextContent("Resolving…");
    // A second click while busy sends nothing more.
    fireEvent.click(busy);
    expect(resolveCalls()).toEqual([
      [
        "resolve_thread",
        { reviewId: t.reviewId, threadId: t.threadId, resolved: true },
      ],
    ]);
    await act(async () => answer({ ...t, resolved: true }));
    expect(
      await screen.findByRole("button", {
        name: "Unresolve bob-fixture's thread",
      }),
    ).toBeEnabled();
    expect(screen.getByText("Resolved")).toBeInTheDocument();
    expect(mockInvoke).toHaveBeenCalledWith("list_threads", {
      reviewId: t.reviewId,
    });
  });

  it("reopens a resolved thread the same way", async () => {
    const done = { ...t, resolved: true };
    seed([done]);
    mockInvoke.mockImplementation((cmd: string) =>
      Promise.resolve(
        cmd === "list_threads" ? [t] : cmd === "resolve_thread" ? t : [],
      ),
    );
    render(<Live id={t.threadId} />);
    fireEvent.click(
      screen.getByRole("button", { name: "Unresolve bob-fixture's thread" }),
    );
    expect(
      await screen.findByRole("button", {
        name: "Resolve bob-fixture's thread",
      }),
    ).toBeEnabled();
    expect(resolveCalls()[0][1]).toMatchObject({ resolved: false });
    expect(screen.queryByText("Resolved")).not.toBeInTheDocument();
  });

  it("says why the platform refused, and stays as it was", async () => {
    mockInvoke.mockImplementation((cmd: string) =>
      cmd === "resolve_thread"
        ? Promise.reject({
            kind: "postRejected",
            message: "GitLab: 403 Forbidden",
          })
        : Promise.resolve([]),
    );
    render(<Live id={t.threadId} />);
    fireEvent.click(
      screen.getByRole("button", { name: "Resolve bob-fixture's thread" }),
    );
    await waitFor(() =>
      expect(useToastStore.getState().toasts).toHaveLength(1),
    );
    expect(useToastStore.getState().toasts[0]).toMatchObject({
      kind: "error",
      message: "Could not resolve the thread",
      detail: "GitLab: 403 Forbidden",
    });
    expect(
      screen.getByRole("button", { name: "Resolve bob-fixture's thread" }),
    ).toBeEnabled();
    expect(screen.queryByText("Resolved")).not.toBeInTheDocument();
  });
});

describe("replying from the thread header", () => {
  const scroll = vi.fn();
  const field = () => screen.getByRole("textbox", { name: "Reply" });
  const replyButton = () =>
    screen.getByRole("button", { name: /^Reply to bob-fixture/ });

  beforeEach(async () => {
    scroll.mockReset();
    Element.prototype.scrollIntoView = scroll;
    const { useCommentStore } = await import("../state/commentStore");
    useCommentStore.setState({ replies: {} });
  });

  it("scrolls to the composer and focuses it", () => {
    render(<Live id={t.threadId} />);
    fireEvent.click(replyButton());
    expect(scroll).toHaveBeenCalledTimes(1);
    expect(scroll.mock.contexts[0]).toBe(
      field().closest('[role="group"]')?.parentElement,
    );
    expect(field()).toHaveFocus();
  });

  it("does it again when the composer is already open", () => {
    render(<Live id={t.threadId} />);
    fireEvent.click(replyButton());
    (document.activeElement as HTMLElement | null)?.blur();
    fireEvent.click(replyButton());
    expect(scroll).toHaveBeenCalledTimes(2);
    expect(field()).toHaveFocus();
  });

  it("does not scroll again while typing", () => {
    render(<Live id={t.threadId} />);
    fireEvent.click(replyButton());
    fireEvent.change(field(), { target: { value: "looks good" } });
    fireEvent.change(field(), { target: { value: "looks good to me" } });
    expect(scroll).toHaveBeenCalledTimes(1);
  });
});

describe("resolved threads fold", () => {
  /** A backend whose `list_threads` reports what `resolve_thread` did. */
  function resolving(start: StoredThread[]) {
    let current = start;
    mockInvoke.mockImplementation(((cmd: string, raw?: unknown) => {
      const a = (raw ?? {}) as Record<string, unknown>;
      if (cmd === "resolve_thread") {
        current = current.map((x) =>
          x.threadId === a.threadId ? { ...x, resolved: !!a.resolved } : x,
        );
        return Promise.resolve(current.find((x) => x.threadId === a.threadId));
      }
      if (cmd === "list_threads") return Promise.resolve(current);
      return Promise.resolve(null);
    }) as typeof invoke);
  }
  const card = (author: string) =>
    screen.getByRole("article", { name: `Thread by ${author}` });

  beforeEach(() => {
    useHistoryStore.setState({ open: {}, expanded: {}, focused: null });
  });

  it("folds once a resolve lands and unfolds when reopened", async () => {
    const mine = thread({
      resolvable: true,
      mine: true,
      author: "rev-fixture",
      notes: [
        {
          id: "n-fold",
          author: "rev-fixture",
          body: "Guard the empty cache.",
          createdAt: "2026-10-15T09:00:00Z",
          system: false,
        },
      ],
    });
    seed([mine]);
    resolving([mine]);
    render(<Live id={mine.threadId} />);
    expect(card("rev-fixture")).toHaveAttribute("data-open", "true");
    fireEvent.click(
      screen.getByRole("button", { name: "Resolve rev-fixture's thread" }),
    );
    // Folded to its header: badge, author, a one-line preview.
    await waitFor(() =>
      expect(card("rev-fixture")).toHaveAttribute("data-open", "false"),
    );
    expect(within(card("rev-fixture")).getByText("Resolved")).toBeVisible();
    expect(
      within(card("rev-fixture")).getByText("Guard the empty cache."),
    ).toBeVisible();
    fireEvent.click(
      screen.getByRole("button", { name: "Unresolve rev-fixture's thread" }),
    );
    await waitFor(() =>
      expect(card("rev-fixture")).toHaveAttribute("data-open", "true"),
    );
  });

  it("start folded, everywhere; a click opens one and it stays open", async () => {
    const done = thread({
      resolvable: true,
      resolved: true,
      mine: true,
      author: "rev-fixture",
    });
    const outdated = thread({
      resolvable: true,
      resolved: true,
      outdated: true,
      mine: true,
      author: "carol-fixture",
    });
    seed([done, outdated]);
    resolving([done, outdated]);
    render(
      <>
        <Live id={done.threadId} />
        <ListedThread thread={outdated} provider="gitlab" now={NOW} />
      </>,
    );
    expect(card("rev-fixture")).toHaveAttribute("data-open", "false");
    expect(card("carol-fixture")).toHaveAttribute("data-open", "false");
    // The user's own resolved one keeps its then/now closed too.
    expect(document.querySelector('[data-slot="then-now"]')).toBeNull();
    fireEvent.click(
      within(card("rev-fixture")).getByRole("button", {
        name: /^rev-fixture/,
        expanded: false,
      }),
    );
    expect(card("rev-fixture")).toHaveAttribute("data-open", "true");
    // A re-read with the same state keeps the user's choice.
    await act(() => useThreadStore.getState().load(done.reviewId));
    expect(card("rev-fixture")).toHaveAttribute("data-open", "true");
  });

  it("reply and then/now unfold a folded one first", () => {
    const done = thread({ resolvable: true, resolved: true });
    const other = thread({
      resolvable: true,
      resolved: true,
      author: "carol-fixture",
    });
    seed([done, other]);
    render(
      <>
        <Live id={done.threadId} />
        <Live id={other.threadId} />
      </>,
    );
    expect(card("bob-fixture")).toHaveAttribute("data-open", "false");
    fireEvent.click(
      screen.getByRole("button", { name: "Reply to bob-fixture's thread" }),
    );
    expect(card("bob-fixture")).toHaveAttribute("data-open", "true");
    expect(card("carol-fixture")).toHaveAttribute("data-open", "false");
    fireEvent.click(
      within(card("carol-fixture")).getByRole("button", {
        name: "Then/now history",
      }),
    );
    expect(card("carol-fixture")).toHaveAttribute("data-open", "true");
  });

  it("the h key on a focused folded one unfolds it with its then/now", () => {
    const done = thread({
      resolvable: true,
      resolved: true,
      mine: true,
      author: "rev-fixture",
    });
    seed([done]);
    render(<Live id={done.threadId} />);
    expect(card("rev-fixture")).toHaveAttribute("data-open", "false");
    act(() => {
      useHistoryStore.getState().focus({
        threadId: done.threadId,
        reviewId: done.reviewId,
        mine: true,
      });
      useHistoryStore.getState().toggleFocused(done.reviewId);
    });
    expect(card("rev-fixture")).toHaveAttribute("data-open", "true");
    expect(document.querySelector('[data-slot="then-now"]')).not.toBeNull();
  });
});

describe("a comment Argus posted outside the diff", () => {
  it("says which lines it is on", () => {
    const base = thread();
    const outside = thread({
      anchor: {
        ...base.anchor!,
        lineNew: 18,
        startLine: 12,
        outsideDiff: true,
      },
    });
    seed([outside]);
    render(<Live id={outside.threadId} />);
    expect(
      screen.getByText("on lines 12–18 (outside the diff)"),
    ).toBeInTheDocument();
  });
});
