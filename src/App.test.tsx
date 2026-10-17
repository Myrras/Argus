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
import { useReviewStore } from "./state/reviewStore";
import { useBackendSettingsStore } from "./state/backendSettingsStore";
import { useSettingsRequestStore } from "./state/settingsRequestStore";
import { review, settings, status } from "./test/fixtures";

const mockInvoke = vi.mocked(invoke);
const initialReviews = useReviewStore.getState();
const initialSettings = useBackendSettingsStore.getState();

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
        case "provider_status":
          return Promise.resolve(
            status({
              gitlab: {
                configured: false,
                baseUrl: "",
                tokenPresent: false,
                problem: null,
              },
            }),
          );
        case "gitlab_token_present":
          return Promise.resolve(false);
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

  it("loads the reviews, the provider status and the settings", async () => {
    render(<App />);
    expect(await screen.findByText("Fixture A")).toBeInTheDocument();
    expect(
      await screen.findByText("GitLab is not set up."),
    ).toBeInTheDocument();
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

  it("opens the provider's tab from the Providers section", async () => {
    render(<App />);
    const providers = screen.getByRole("region", { name: "Providers" });
    await within(providers).findByText("GitLab is not set up.");
    fireEvent.click(
      within(providers).getByRole("button", { name: "Settings" }),
    );
    const dialog = await screen.findByRole("dialog", { name: "Settings" });
    expect(within(dialog).getByRole("tab", { name: "GitLab" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });
});
