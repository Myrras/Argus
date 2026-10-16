import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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
import { useToastStore } from "../state/toastStore";
import { review } from "../test/fixtures";
import { ReviewList } from "./ReviewList";
import { Toasts } from "./Toasts";

const mockInvoke = vi.mocked(invoke);
const initial = useReviewStore.getState();
const NOW = Date.parse("2026-10-16T12:00:00Z");

const fresh = review({ title: "First look", status: "needsReview" });
const waiting = review({ title: "Waiting", status: "waitingOnAuthor" });

function openMenu(title: string) {
  fireEvent.contextMenu(screen.getByText(title), { clientX: 40, clientY: 60 });
  return screen.getByRole("menu");
}

describe("ReviewList context menu", () => {
  const clipboard = { writeText: vi.fn() };

  beforeEach(() => {
    useReviewStore.setState(
      { ...initial, reviews: [fresh, waiting], loaded: true },
      true,
    );
    useToastStore.setState({ toasts: [] });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: clipboard,
    });
    mockInvoke.mockImplementation(((cmd: string) =>
      Promise.resolve(
        cmd === "mark_reviewed"
          ? { ...fresh, status: "waitingOnAuthor" }
          : cmd === "list_reviews"
            ? [fresh, waiting]
            : null,
      )) as typeof invoke);
  });

  afterEach(() => {
    Reflect.deleteProperty(navigator, "clipboard");
  });

  it("opens the review in the browser", () => {
    render(<ReviewList now={NOW} />);
    const menu = openMenu("First look");
    expect(menu).toHaveStyle({ left: "40px", top: "60px" });
    fireEvent.click(
      within(menu).getByRole("menuitem", { name: "Open in GitHub" }),
    );
    expect(openUrl).toHaveBeenCalledWith(fresh.url);
    // Choosing an item closes the menu.
    return waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  });

  it("copies the link and says so, or says it could not", async () => {
    render(
      <>
        <ReviewList now={NOW} />
        <Toasts />
      </>,
    );
    clipboard.writeText.mockResolvedValueOnce(undefined);
    fireEvent.click(
      within(openMenu("First look")).getByRole("menuitem", {
        name: "Copy link",
      }),
    );
    expect(clipboard.writeText).toHaveBeenCalledWith(fresh.url);
    expect(await screen.findByText("Link copied")).toBeInTheDocument();

    clipboard.writeText.mockRejectedValueOnce(new Error("denied"));
    fireEvent.click(
      within(openMenu("First look")).getByRole("menuitem", {
        name: "Copy link",
      }),
    );
    expect(
      await screen.findByText("Could not copy the link"),
    ).toBeInTheDocument();
  });

  it("marks a round reviewed only while one is open", async () => {
    render(<ReviewList now={NOW} />);
    act(() => useReviewStore.getState().setTab("waitingOnAuthor"));
    expect(
      within(openMenu("Waiting")).getByRole("menuitem", {
        name: "Mark round reviewed",
      }),
    ).toBeDisabled();
    act(() => {
      fireEvent.keyDown(window, { key: "Escape" });
    });
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    act(() => useReviewStore.getState().setTab("needsReview"));

    fireEvent.click(
      within(openMenu("First look")).getByRole("menuitem", {
        name: "Mark round reviewed",
      }),
    );
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("mark_reviewed", {
        reviewId: fresh.id,
      }),
    );
    await waitFor(() => expect(useReviewStore.getState().marking).toBeNull());
  });

  it("closes on a click or right-click outside it", async () => {
    render(<ReviewList now={NOW} />);
    openMenu("First look");
    const backdrop = screen.getByRole("menu").parentElement!;
    fireEvent.mouseDown(screen.getByRole("menu"));
    expect(screen.getByRole("menu")).toBeInTheDocument();
    fireEvent.mouseDown(backdrop);
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());

    openMenu("First look");
    fireEvent.contextMenu(screen.getByRole("menu").parentElement!);
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  });
});
