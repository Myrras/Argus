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
const events = vi.hoisted(() => {
  const handlers = new Map<string, (() => void)[]>();
  return {
    listen: (name: string, cb: () => void) => {
      handlers.set(name, [...(handlers.get(name) ?? []), cb]);
      return Promise.resolve(() => undefined);
    },
    emit: (name: string) => (handlers.get(name) ?? []).forEach((h) => h()),
    clear: () => handlers.clear(),
  };
});
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: () => void) => events.listen(name, cb)),
}));

import { invoke } from "@tauri-apps/api/core";

import App from "./App";
import type { RefreshError, RefreshReport } from "./ipc/types";
import { useReviewStore } from "./state/reviewStore";
import { useBackendSettingsStore } from "./state/backendSettingsStore";
import { useSettingsRequestStore } from "./state/settingsRequestStore";
import { account, review, settings } from "./test/fixtures";

const mockInvoke = vi.mocked(invoke);
const initialReviews = useReviewStore.getState();
const initialSettings = useBackendSettingsStore.getState();

/** glab knows the host but has no login for it. */
const glabLoggedOut = account({
  provider: "gitlab",
  host: "gitlab.example.test",
  user: null,
  state: { kind: "notLoggedIn", message: null },
});
const NOT_LOGGED_IN =
  "GitLab unavailable: glab is not logged in to gitlab.example.test. Run glab auth login --hostname gitlab.example.test.";

const a = review({ title: "Fixture A" });
const b = review({ title: "Fixture B" });

describe("App", () => {
  beforeEach(() => {
    useReviewStore.setState({ ...initialReviews }, true);
    useBackendSettingsStore.setState({ ...initialSettings }, true);
    useSettingsRequestStore.getState().clear();
    events.clear();
    mockInvoke.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "list_reviews":
          return Promise.resolve([a, b]);
        case "last_refresh":
          return Promise.resolve(null);
        case "get_settings":
          return Promise.resolve(settings());
        case "list_accounts":
          return Promise.resolve([account(), glabLoggedOut]);
        case "refresh_reviews":
          return Promise.resolve({
            changed: [],
            errors: [],
            refreshedAt: new Date().toISOString(),
          });
        case "list_threads":
          return Promise.resolve([]);
        default:
          return Promise.resolve(null);
      }
    });
  });

  it("loads the reviews, the accounts and the settings", async () => {
    render(<App />);
    expect(await screen.findByText("Fixture A")).toBeInTheDocument();
    expect(await screen.findByText(NOT_LOGGED_IN)).toBeInTheDocument();
    expect(mockInvoke).toHaveBeenCalledWith("list_accounts", {
      recheck: false,
    });
    expect(screen.getByText("Refreshed never")).toBeInTheDocument();
    // The island shell: sidebar and the welcome screen in the detail island.
    expect(
      screen.getByRole("complementary", { name: "Sidebar" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Welcome to Argus")).toBeInTheDocument();
  });

  it("drives the list from the keyboard", async () => {
    render(<App />);
    await screen.findByText("Fixture A");
    fireEvent.keyDown(window, { key: "ArrowDown" });
    fireEvent.keyDown(window, { key: "ArrowDown" });
    fireEvent.keyDown(window, { key: "Enter" });
    expect(
      await screen.findByRole("heading", { name: "Fixture B" }),
    ).toBeInTheDocument();

    // Refresh is ⇧R since M6 (bare R replies to the focused thread).
    fireEvent.keyDown(window, { key: "R", shiftKey: true });
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("refresh_reviews"),
    );
    expect(await screen.findByText("Refreshed just now")).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "3" });
    expect(useReviewStore.getState().tab).toBe("waitingOnAuthor");
  });

  it("opens settings from the title bar", async () => {
    render(<App />);
    fireEvent.click(screen.getByTitle("Settings (Ctrl+,)"));
    expect(
      await screen.findByRole("dialog", { name: "Settings" }),
    ).toBeInTheDocument();
  });

  it("opens settings from the native menu event", async () => {
    render(<App />);
    await screen.findByText("Fixture A");
    act(() => events.emit("menu:open-settings"));
    expect(
      await screen.findByRole("dialog", { name: "Settings" }),
    ).toBeInTheDocument();
  });

  it("drops a GitLab error from before its host was switched", async () => {
    // A refresh from before the host was switched off: its report says
    // the host failed.
    const failed: RefreshError = {
      provider: "gitlab",
      host: "gitlab.example.test",
      review: null,
      message: "Could not fetch reviews: glab could not reach GitLab",
      unavailable: false,
      rateLimitedUntil: null,
    };
    const stale: RefreshReport = {
      changed: [],
      errors: [failed],
      refreshedAt: new Date(Date.now() - 1000).toISOString(),
    };
    let stored = settings();
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
      switch (cmd) {
        case "last_refresh":
          return Promise.resolve(stale);
        case "get_settings":
          return Promise.resolve(stored);
        case "update_settings":
          stored = (args as { settings: typeof stored }).settings;
          return Promise.resolve(stored);
        case "list_accounts":
          return Promise.resolve([
            account(),
            account({
              provider: "gitlab",
              host: "gitlab.example.test",
              enabled:
                stored.accounts["gitlab:gitlab.example.test"]?.enabled ?? true,
            }),
          ]);
        default:
          return base(cmd);
      }
    });
    render(<App />);
    const providers = screen.getByRole("region", { name: "Providers" });
    const shown = `GitLab (gitlab.example.test): ${failed.message}`;
    await within(providers).findByText(shown);

    fireEvent.click(screen.getByTitle("Settings (Ctrl+,)"));
    const dialog = await screen.findByRole("dialog", { name: "Settings" });
    fireEvent.click(within(dialog).getByRole("tab", { name: "Accounts" }));
    fireEvent.click(
      await within(dialog).findByRole("switch", {
        name: "Use GitLab at gitlab.example.test",
      }),
    );
    await waitFor(() =>
      expect(stored.accounts["gitlab:gitlab.example.test"]).toEqual({
        enabled: false,
        account: null,
      }),
    );
    // Only GitHub is in use now, and the old report's error is gone.
    await waitFor(() =>
      expect(within(providers).getAllByText("Connected")).toHaveLength(1),
    );
    expect(useReviewStore.getState().lastRefresh).toEqual(stale);
    expect(within(providers).queryByText(shown)).toBeNull();

    // A refresh that began after the change does show its errors.
    act(() =>
      useReviewStore.setState({
        lastRefresh: {
          ...stale,
          refreshedAt: new Date(Date.now() + 1000).toISOString(),
        },
      }),
    );
    expect(within(providers).getByText(shown)).toBeInTheDocument();
  });

  it("opens Settings → Accounts on the host from the Providers section", async () => {
    render(<App />);
    const providers = screen.getByRole("region", { name: "Providers" });
    await within(providers).findByText(NOT_LOGGED_IN);
    fireEvent.click(
      within(providers).getByRole("button", { name: "Settings" }),
    );
    const dialog = await screen.findByRole("dialog", { name: "Settings" });
    expect(
      within(dialog).getByRole("tab", { name: "Accounts" }),
    ).toHaveAttribute("aria-selected", "true");
    const card = within(dialog).getByRole("group", {
      name: "GitLab gitlab.example.test",
    });
    await waitFor(() =>
      expect(card).toHaveAttribute("data-highlighted", "true"),
    );
    expect(card).toHaveTextContent(
      "glab auth login --hostname gitlab.example.test",
    );
  });
});
