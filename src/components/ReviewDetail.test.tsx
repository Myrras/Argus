import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn(() => Promise.resolve()),
}));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

import { useReviewStore } from "../state/reviewStore";
import { useThreadStore } from "../state/threadStore";
import { review, thread } from "../test/fixtures";
import { ReviewDetail } from "./ReviewDetail";

const mockInvoke = vi.mocked(invoke);
const initialReviews = useReviewStore.getState();
const initialThreads = useThreadStore.getState();
const NOW = Date.parse("2026-10-16T12:00:00Z");

const r = review({
  title: "Add widget caching",
  status: "needsReReview",
  round: 3,
  roundReasons: ["newCommits"],
  untouchedThreads: 1,
});

const threads = [
  thread({
    reviewId: r.id,
    author: "bob-fixture",
    createdAt: "2026-10-14T08:00:00Z",
    anchor: { ...thread().anchor!, pathNew: "src/other.rs", lineNew: 7 },
  }),
  thread({
    reviewId: r.id,
    mine: true,
    author: "rev-fixture",
    createdAt: "2026-10-15T08:00:00Z",
    outdated: true,
    anchor: { ...thread().anchor!, pathNew: "src/cache.rs", lineNew: 42 },
  }),
  thread({
    reviewId: r.id,
    mine: true,
    author: "rev-fixture",
    createdAt: "2026-10-13T08:00:00Z",
    resolved: true,
    anchor: null,
  }),
];

describe("ReviewDetail", () => {
  beforeEach(() => {
    mockInvoke.mockImplementation((cmd: string) =>
      Promise.resolve(
        cmd === "list_threads" ? threads : cmd === "list_reviews" ? [r] : r,
      ),
    );
    useReviewStore.setState(
      { ...initialReviews, reviews: [r], loaded: true, selectedId: r.id },
      true,
    );
    useThreadStore.setState({ ...initialThreads, byReview: {} }, true);
  });

  it("shows a placeholder without a selection", () => {
    useReviewStore.setState({ ...initialReviews, reviews: [r] }, true);
    render(<ReviewDetail now={NOW} />);
    expect(
      screen.getByText("Select a review to see its threads."),
    ).toBeInTheDocument();
  });

  it("lists the user's threads first, outdated ones included", async () => {
    render(<ReviewDetail now={NOW} />);
    expect(mockInvoke).toHaveBeenCalledWith("list_threads", {
      reviewId: r.id,
    });
    const cards = await screen.findAllByRole("article");
    // Only the user's threads show until "Other threads" is expanded; mine
    // are oldest first.
    expect(cards.map((c) => c.getAttribute("aria-label"))).toEqual([
      "Thread on PR-level",
      "Thread on src/cache.rs:42",
    ]);
    const outdated = cards[1];
    expect(within(outdated).getByText("Outdated")).toBeInTheDocument();
    expect(within(outdated).getByText("Open")).toBeInTheDocument();
    expect(within(cards[0]).getByText("Resolved")).toBeInTheDocument();
    // The then/now slot is there for anchored threads only.
    expect(outdated.querySelector('[data-slot="then-now"]')).not.toBeNull();
    expect(cards[0].querySelector('[data-slot="then-now"]')).toBeNull();

    const toggle = screen.getByRole("button", { name: /Other threads/ });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(toggle);
    const all = screen.getAllByRole("article");
    expect(all).toHaveLength(3);
    expect(all[2]).toHaveAttribute("aria-label", "Thread on src/other.rs:7");
  });

  it("shows status, round and reasons, and marks the round reviewed", async () => {
    render(<ReviewDetail now={NOW} />);
    expect(
      screen.getByRole("heading", { name: "Add widget caching" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Round 3")).toBeInTheDocument();
    expect(screen.getByText("Needs re-review")).toBeInTheDocument();
    expect(screen.getByText("New commits")).toBeInTheDocument();
    expect(screen.getByText("1 of your threads untouched")).toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", { name: "Mark round reviewed" }),
    );
    expect(mockInvoke).toHaveBeenCalledWith("mark_reviewed", {
      reviewId: r.id,
    });
    await screen.findAllByRole("article");
  });

  it("opens the review in the browser", async () => {
    render(<ReviewDetail now={NOW} />);
    fireEvent.click(screen.getByRole("button", { name: /Open in GitHub/ }));
    expect(openUrl).toHaveBeenCalledWith(r.url);
    await screen.findAllByRole("article");
  });

  it("disables marking when nothing is new", async () => {
    const waiting = {
      ...r,
      status: "waitingOnAuthor" as const,
      roundReasons: [],
    };
    useReviewStore.setState(
      { ...initialReviews, reviews: [waiting], selectedId: waiting.id },
      true,
    );
    render(<ReviewDetail now={NOW} />);
    expect(
      screen.getByRole("button", { name: "Mark round reviewed" }),
    ).toBeDisabled();
    await screen.findAllByRole("article");
  });
});
