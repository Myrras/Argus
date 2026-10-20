import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";

import type { StoredThread, ThreadId } from "../ipc/types";
import { useThreadStore } from "../state/threadStore";
import { useToastStore } from "../state/toastStore";
import { thread } from "../test/fixtures";
import { ThreadBlock } from "./ThreadBlock";

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
