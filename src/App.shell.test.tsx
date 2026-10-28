/**
 * The App shell's own behaviour: the webview's context menu, the sidebar
 * resizer, failures while starting up, live events from the backend, and
 * unsubscribing on unmount.
 */

import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const events = vi.hoisted(() => {
  const handlers = new Map<string, ((e: { payload: unknown }) => void)[]>();
  return {
    listen: (name: string, cb: (e: { payload: unknown }) => void) => {
      handlers.set(name, [...(handlers.get(name) ?? []), cb]);
      return Promise.resolve(() => {
        handlers.set(
          name,
          (handlers.get(name) ?? []).filter((h) => h !== cb),
        );
      });
    },
    emit: (name: string, payload: unknown) =>
      (handlers.get(name) ?? []).forEach((h) => h({ payload })),
    count: () => [...handlers.values()].reduce((n, list) => n + list.length, 0),
    clear: () => handlers.clear(),
  };
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) =>
    events.listen(name, cb),
  ),
}));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";

import App from "./App";
import type { ReviewRecord } from "./ipc/types";
import { useBackendSettingsStore } from "./state/backendSettingsStore";
import { useLayoutStore } from "./state/layoutStore";
import { useReviewStore } from "./state/reviewStore";
import { useToastStore } from "./state/toastStore";
import { review, settings, status } from "./test/fixtures";

const mockInvoke = vi.mocked(invoke);
const initialReviews = useReviewStore.getState();
const initialSettings = useBackendSettingsStore.getState();
const initialLayout = useLayoutStore.getState();

function backend(over: Record<string, () => unknown> = {}) {
  const base: Record<string, () => unknown> = {
    list_reviews: () => [],
    last_refresh: () => null,
    get_settings: () => settings(),
    provider_status: () => status(),
    gitlab_token_present: () => true,
  };
  const all = { ...base, ...over };
  mockInvoke.mockImplementation(((cmd: string) => {
    try {
      return Promise.resolve(all[cmd]?.() ?? null);
    } catch (e) {
      return Promise.reject(e);
    }
  }) as typeof invoke);
}

const notifications = () =>
  screen.findByRole("region", { name: "Notifications" });

describe("App shell", () => {
  beforeEach(() => {
    localStorage.clear();
    events.clear();
    useReviewStore.setState({ ...initialReviews }, true);
    useBackendSettingsStore.setState({ ...initialSettings }, true);
    useLayoutStore.setState({ ...initialLayout, sidebarWidth: 340 }, true);
    useToastStore.setState({ toasts: [] });
  });

  it("keeps the browser's context menu away, except in text fields", async () => {
    backend();
    render(
      <>
        <App />
        <textarea aria-label="Field" />
      </>,
    );
    await screen.findByText(/Refreshed/);
    const outside = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
    });
    screen.getByText("Argus").dispatchEvent(outside);
    expect(outside.defaultPrevented).toBe(true);
    const inField = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
    });
    screen.getByRole("textbox", { name: "Field" }).dispatchEvent(inField);
    expect(inField.defaultPrevented).toBe(false);
  });

  it("resizes the sidebar by dragging, within its limits", async () => {
    backend();
    render(<App />);
    const handle = screen.getByRole("separator", { name: "Resize sidebar" });
    fireEvent.mouseDown(handle, { clientX: 340 });
    expect(document.body.style.cursor).toBe("col-resize");
    fireEvent.mouseMove(window, { clientX: 400 });
    expect(useLayoutStore.getState().sidebarWidth).toBe(400);
    fireEvent.mouseMove(window, { clientX: 2000 });
    expect(useLayoutStore.getState().sidebarWidth).toBe(600);
    fireEvent.mouseUp(window);
    expect(document.body.style.cursor).toBe("");
    // Released: further moves do nothing.
    fireEvent.mouseMove(window, { clientX: 300 });
    expect(useLayoutStore.getState().sidebarWidth).toBe(600);
  });

  it("says what failed while starting up", async () => {
    const fail = (message: string) => () => {
      throw { kind: "store", message };
    };
    backend({
      list_reviews: fail("The database is locked."),
      get_settings: fail("settings.json is unreadable."),
      provider_status: fail("gh did not answer."),
    });
    render(<App />);
    const region = await notifications();
    expect(
      await within(region).findByText("Could not load the reviews"),
    ).toBeInTheDocument();
    expect(
      within(region).getByText("The database is locked."),
    ).toBeInTheDocument();
    expect(
      await within(region).findByText("Could not load the settings"),
    ).toBeInTheDocument();
    expect(
      await within(region).findByText("Could not check the providers"),
    ).toBeInTheDocument();
    // The list still renders, empty.
    expect(useReviewStore.getState().loaded).toBe(true);
  });

  it("follows refreshes and review changes from the backend", async () => {
    let reviews: ReviewRecord[] = [review({ title: "Before" })];
    backend({
      list_reviews: () => reviews,
      get_review: () => ({ ...reviews[0], title: "Renamed upstream" }),
    });
    render(<App />);
    expect(await screen.findByText("Before")).toBeInTheDocument();

    await act(async () => {
      events.emit("review:changed", { reviewId: reviews[0].id });
    });
    expect(await screen.findByText("Renamed upstream")).toBeInTheDocument();

    reviews = [...reviews, review({ title: "New arrival" })];
    await act(async () => {
      events.emit("reviews:refreshed", {
        changed: [reviews[1].id],
        errors: [],
        refreshedAt: new Date().toISOString(),
      });
    });
    expect(await screen.findByText("New arrival")).toBeInTheDocument();
    expect(screen.getByText("Refreshed just now")).toBeInTheDocument();
  });

  it("unsubscribes from every event when it goes away", async () => {
    backend();
    const { unmount } = render(<App />);
    await screen.findByText(/Refreshed/);
    await act(async () => undefined);
    expect(events.count()).toBeGreaterThan(0);
    unmount();
    await act(async () => undefined);
    expect(events.count()).toBe(0);
  });
});
