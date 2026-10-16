import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));

import { invoke } from "@tauri-apps/api/core";

import App from "./App";
import { useReviewStore } from "./state/reviewStore";
import { useSettingsStore } from "./state/settingsStore";
import { review, settings, status } from "./test/fixtures";

const mockInvoke = vi.mocked(invoke);
const initialReviews = useReviewStore.getState();
const initialSettings = useSettingsStore.getState();

const a = review({ title: "Fixture A" });
const b = review({ title: "Fixture B" });

describe("App", () => {
  beforeEach(() => {
    useReviewStore.setState({ ...initialReviews }, true);
    useSettingsStore.setState({ ...initialSettings }, true);
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
              gitlab: { configured: false, baseUrl: "", tokenPresent: false },
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

    fireEvent.keyDown(window, { key: "r" });
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("refresh_reviews"),
    );
    expect(await screen.findByText("Refreshed just now")).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "3" });
    expect(useReviewStore.getState().tab).toBe("waitingOnAuthor");
  });

  it("opens settings from the title bar", async () => {
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(
      await screen.findByRole("dialog", { name: "Settings" }),
    ).toBeInTheDocument();
  });
});
