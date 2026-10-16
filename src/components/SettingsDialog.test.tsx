import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";

import type { Settings } from "../ipc/types";
import { useSettingsStore } from "../state/settingsStore";
import { settings, status } from "../test/fixtures";
import { SettingsDialog } from "./SettingsDialog";

const mockInvoke = vi.mocked(invoke);
const initial = useSettingsStore.getState();
const TOKEN = "glpat-fixture-not-a-real-token";

function backend(stored: Settings, tokenPresent: boolean) {
  mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
    switch (cmd) {
      case "update_settings":
        return Promise.resolve((args as { settings: Settings }).settings);
      case "set_gitlab_token":
        return Promise.resolve(undefined);
      case "gitlab_token_present":
        return Promise.resolve(tokenPresent);
      case "provider_status":
        return Promise.resolve(status());
      default:
        return Promise.resolve(stored);
    }
  });
}

function seed(s: Settings, tokenPresent: boolean) {
  useSettingsStore.setState(
    { ...initial, settings: s, tokenPresent, status: status() },
    true,
  );
}

describe("SettingsDialog", () => {
  beforeEach(() => {
    const s = settings();
    backend(s, false);
    seed(s, false);
  });

  it("sends the token only to set_gitlab_token, after the settings", async () => {
    const onClose = vi.fn();
    render(<SettingsDialog open onClose={onClose} />);
    expect(screen.getByText("Not set")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "https://gitlab.example.test/" },
    });
    fireEvent.change(screen.getByLabelText("Personal access token"), {
      target: { value: TOKEN },
    });
    fireEvent.change(screen.getByLabelText("Account (optional)"), {
      target: { value: "work-fixture" },
    });
    fireEvent.change(screen.getByLabelText("Refresh every"), {
      target: { value: "5" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(onClose).toHaveBeenCalled());
    const calls = mockInvoke.mock.calls.map(([cmd]) => cmd);
    expect(calls.indexOf("update_settings")).toBeLessThan(
      calls.indexOf("set_gitlab_token"),
    );
    const update = mockInvoke.mock.calls.find(
      ([cmd]) => cmd === "update_settings",
    );
    expect(JSON.stringify(update)).not.toContain(TOKEN);
    expect(update?.[1]).toEqual({
      settings: {
        ...settings(),
        github: { enabled: true, host: "github.com", account: "work-fixture" },
        gitlab: { baseUrl: "https://gitlab.example.test/" },
        pollIntervalSecs: 300,
      },
    });
    expect(mockInvoke).toHaveBeenCalledWith("set_gitlab_token", {
      token: TOKEN,
    });
  });

  it("keeps the stored token unless it is replaced or removed", async () => {
    const s = settings({ gitlab: { baseUrl: "https://gitlab.example.test" } });
    backend(s, true);
    seed(s, true);
    const onClose = vi.fn();
    const { unmount } = render(<SettingsDialog open onClose={onClose} />);
    expect(screen.getByText("Token set")).toBeInTheDocument();
    // The stored token is never shown.
    expect(screen.getByLabelText("Personal access token")).toHaveValue("");

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(
      mockInvoke.mock.calls.some(([cmd]) => cmd === "set_gitlab_token"),
    ).toBe(false);
    unmount();

    mockInvoke.mockClear();
    const onClose2 = vi.fn();
    render(<SettingsDialog open onClose={onClose2} />);
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(screen.getByText("Will be removed")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onClose2).toHaveBeenCalled());
    expect(mockInvoke).toHaveBeenCalledWith("set_gitlab_token", { token: "" });
  });

  it("refuses a token without a GitLab URL and shows backend errors", async () => {
    const onClose = vi.fn();
    render(<SettingsDialog open onClose={onClose} />);
    fireEvent.change(screen.getByLabelText("Personal access token"), {
      target: { value: TOKEN },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(
      await screen.findByText("Set the GitLab URL before its token."),
    ).toBeInTheDocument();
    expect(mockInvoke).not.toHaveBeenCalled();

    mockInvoke.mockRejectedValue({
      kind: "invalidInput",
      message:
        "invalid input: the GitLab URL must start with https:// or http://",
    });
    fireEvent.change(screen.getByLabelText("Personal access token"), {
      target: { value: "" },
    });
    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "gitlab.example.test" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(
      await screen.findByText(/must start with https:\/\//),
    ).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
  });
});
