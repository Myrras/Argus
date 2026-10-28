import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { useReviewStore } from "../state/reviewStore";
import { review } from "../test/fixtures";
import { ReviewList } from "./ReviewList";

const initial = useReviewStore.getState();
const NOW = Date.parse("2026-10-16T12:00:00Z");

const reviews = [
  review({ title: "First look", status: "needsReview" }),
  review({ title: "Another first look", status: "needsReview" }),
  review({
    title: "Pushed again",
    status: "needsReReview",
    round: 2,
    roundReasons: ["newCommits", "authorReplied", "reRequested"],
    untouchedThreads: 2,
  }),
  review({ title: "Waiting", status: "waitingOnAuthor" }),
  review({
    title: "Merged anyway",
    status: "closedWithoutApproval",
    state: "merged",
    provider: "gitlab",
    host: "gitlab.example.test",
    repo: "grp/proj",
    number: 31,
  }),
];

function seed(tab = initial.tab) {
  useReviewStore.setState({ ...initial, reviews, loaded: true, tab }, true);
}

describe("ReviewList", () => {
  beforeEach(() => seed());

  it("shows each tab's count and only that tab's reviews", () => {
    render(<ReviewList now={NOW} />);
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((t) => t.textContent)).toEqual([
      "Needs review2",
      "Needs re-review1",
      "Waiting on author1",
      "Closed without my approval1",
    ]);
    expect(screen.getByText("First look")).toBeInTheDocument();
    expect(screen.getByText("Another first look")).toBeInTheDocument();
    expect(screen.queryByText("Pushed again")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: /Closed without/ }));
    expect(screen.getByText("Merged anyway")).toBeInTheDocument();
    expect(screen.queryByText("First look")).not.toBeInTheDocument();
    // GitLab refs use `!`, and the platform is visible.
    expect(screen.getByText("grp/proj!31")).toBeInTheDocument();
    expect(screen.getByText("GitLab")).toBeInTheDocument();
  });

  it("renders the re-review reasons and untouched threads", () => {
    seed("needsReReview");
    render(<ReviewList now={NOW} />);
    const item = screen.getByRole("button", { name: /Pushed again/ });
    expect(within(item).getByText("New commits")).toBeInTheDocument();
    expect(within(item).getByText("Author replied")).toBeInTheDocument();
    expect(within(item).getByText("Re-requested")).toBeInTheDocument();
    expect(
      within(item).getByText("2 of your threads untouched"),
    ).toBeInTheDocument();
    expect(within(item).getByText("alice-fixture")).toBeInTheDocument();
    expect(within(item).getByText("2h")).toBeInTheDocument();
  });

  it("has an empty state per tab", () => {
    useReviewStore.setState(
      { ...initial, reviews: [], loaded: true, tab: "waitingOnAuthor" },
      true,
    );
    render(<ReviewList now={NOW} />);
    expect(
      screen.getByText("Nothing is waiting on an author."),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: /Needs re-review/ }));
    expect(
      screen.getByText("No reviews need another look."),
    ).toBeInTheDocument();
  });

  it("selects a review on click", () => {
    render(<ReviewList now={NOW} />);
    fireEvent.click(screen.getByRole("button", { name: /First look/ }));
    expect(useReviewStore.getState().selectedId).toBe(reviews[0].id);
  });
});
