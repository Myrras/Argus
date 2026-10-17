import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(() => Promise.resolve("/src/widgets")),
}));
vi.mock("@tauri-apps/plugin-log", () => ({
  error: vi.fn(() => Promise.resolve()),
  warn: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

import type { Settings } from "../ipc/types";
import { useAiStore } from "../state/aiStore";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { useSettingsStore } from "../state/settingsStore";
import { useThemeStore } from "../state/themeStore";
import { useToastStore } from "../state/toastStore";
import { settings, status } from "../test/fixtures";
import { SettingsModal } from "./SettingsModal";

const mockInvoke = vi.mocked(invoke);
const initialBackend = useBackendSettingsStore.getState();
const TOKEN = "glpat-fixture-not-a-real-token";

/** A fake core: `update_settings` stores what it gets. */
function backend(initial: Settings, tokenPresent: boolean) {
  let stored = initial;
  mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
    switch (cmd) {
      case "get_settings":
        return Promise.resolve(stored);
      case "update_settings":
        stored = (args as { settings: Settings }).settings;
        return Promise.resolve(stored);
      case "set_gitlab_token":
        tokenPresent = (args as { token: string }).token !== "";
        return Promise.resolve(undefined);
      case "gitlab_token_present":
        return Promise.resolve(tokenPresent);
      case "provider_status":
        return Promise.resolve(status());
      default:
        return Promise.resolve(undefined);
    }
  });
}

/** The `settings` of every `update_settings` call so far. */
const updates = () =>
  mockInvoke.mock.calls
    .filter(([cmd]) => cmd === "update_settings")
    .map(([, args]) => (args as { settings: Settings }).settings);

async function renderTab(name: string) {
  render(<SettingsModal open onClose={() => undefined} />);
  fireEvent.click(screen.getByRole("tab", { name }));
  await waitFor(() =>
    expect(useBackendSettingsStore.getState().settings).not.toBeNull(),
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  useSettingsStore.getState().resetSettings();
  useThemeStore.getState().setTheme("dark");
  useSettingsRequestStore.getState().clear();
  useBackendSettingsStore.setState({ ...initialBackend }, true);
  useToastStore.setState({ toasts: [] });
  backend(settings(), false);
});

describe("SettingsModal — General tab", () => {
  it("opens on the General tab", () => {
    render(<SettingsModal open onClose={() => undefined} />);
    expect(screen.getByRole("tab", { name: "General" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByText("Appearance")).toBeInTheDocument();
  });

  it("switches the theme and marks the active one", () => {
    render(<SettingsModal open onClose={() => undefined} />);
    fireEvent.click(screen.getByRole("button", { name: "Retro" }));
    expect(useThemeStore.getState().mode).toBe("retro");
    expect(localStorage.getItem("argus.theme")).toBe("retro");
    expect(screen.getByRole("button", { name: "Retro" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("button", { name: "Dark" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("toggles reduced motion", () => {
    render(<SettingsModal open onClose={() => undefined} />);
    fireEvent.click(screen.getByRole("switch", { name: "Reduce motion" }));
    expect(useSettingsStore.getState().appearance.reduceMotion).toBe(true);
    expect(document.documentElement.dataset.reduceMotion).toBe("true");
  });

  it("steps the refresh interval in minutes and saves it at once", async () => {
    render(<SettingsModal open onClose={() => undefined} />);
    await waitFor(() =>
      expect(screen.getByLabelText("Refresh every")).toHaveTextContent("3 min"),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Increase refresh every" }),
    );
    await waitFor(() =>
      expect(screen.getByLabelText("Refresh every")).toHaveTextContent("4 min"),
    );
    expect(updates()).toEqual([{ ...settings(), pollIntervalSecs: 240 }]);
  });

  it("opens on the tab a settings request asks for", () => {
    useSettingsRequestStore.getState().open("gitlab");
    render(<SettingsModal open onClose={() => undefined} />);
    expect(screen.getByRole("tab", { name: "GitLab" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });
});

describe("SettingsModal — GitHub tab", () => {
  it("turns GitHub off at once", async () => {
    await renderTab("GitHub");
    const toggle = screen.getByRole("switch", { name: "Use GitHub" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(toggle).toHaveAttribute("aria-checked", "false"),
    );
    expect(updates()).toEqual([
      { ...settings(), github: { ...settings().github, enabled: false } },
    ]);
  });

  it("saves the host and account from the form", async () => {
    await renderTab("GitHub");
    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Account (optional)"), {
      target: { value: " work-fixture " },
    });
    fireEvent.click(save);
    await waitFor(() =>
      expect(updates()).toEqual([
        {
          ...settings(),
          github: {
            enabled: true,
            host: "github.com",
            account: "work-fixture",
          },
        },
      ]),
    );
    await waitFor(() =>
      expect(useToastStore.getState().toasts).toMatchObject([
        { kind: "success", message: "GitHub settings saved" },
      ]),
    );
  });
});

describe("SettingsModal — GitLab tab", () => {
  it("sends the token only to set_gitlab_token, after the settings", async () => {
    await renderTab("GitLab");
    expect(screen.getByText("Not set")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "https://gitlab.example.test" },
    });
    fireEvent.change(screen.getByLabelText("Personal access token"), {
      target: { value: TOKEN },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(screen.getByText("Token set")).toBeVisible());
    const calls = mockInvoke.mock.calls.map(([cmd]) => cmd);
    expect(calls.indexOf("update_settings")).toBeLessThan(
      calls.indexOf("set_gitlab_token"),
    );
    expect(JSON.stringify(updates())).not.toContain(TOKEN);
    expect(updates()).toEqual([
      { ...settings(), gitlab: { baseUrl: "https://gitlab.example.test" } },
    ]);
    expect(mockInvoke).toHaveBeenCalledWith("set_gitlab_token", {
      token: TOKEN,
    });
    // The field never shows the token back.
    expect(screen.getByLabelText("Personal access token")).toHaveValue("");
  });

  it("keeps the stored token unless it is replaced or removed", async () => {
    backend(
      settings({ gitlab: { baseUrl: "https://gitlab.example.test" } }),
      true,
    );
    await renderTab("GitLab");
    await waitFor(() => expect(screen.getByText("Token set")).toBeVisible());
    expect(screen.getByLabelText("Personal access token")).toHaveValue("");
    // Nothing changed: nothing to save, and the token is kept.
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Remove token" }));
    await waitFor(() => expect(screen.getByText("Not set")).toBeVisible());
    expect(mockInvoke).toHaveBeenCalledWith("set_gitlab_token", { token: "" });
    expect(updates()).toEqual([]);
  });

  it("refuses a token without a GitLab URL, and toasts backend errors", async () => {
    await renderTab("GitLab");
    fireEvent.change(screen.getByLabelText("Personal access token"), {
      target: { value: TOKEN },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Set the GitLab URL before its token.",
    );
    expect(
      mockInvoke.mock.calls.some(([cmd]) => cmd === "set_gitlab_token"),
    ).toBe(false);

    mockInvoke.mockImplementationOnce(() =>
      Promise.reject({
        kind: "invalidInput",
        message: "the GitLab URL must start with https:// or http://",
      }),
    );
    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "gitlab.example.test" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(useToastStore.getState().toasts).toMatchObject([
        {
          kind: "error",
          message: "Could not save the settings",
          detail: "the GitLab URL must start with https:// or http://",
        },
      ]),
    );
    // A failed save stores no token.
    expect(
      mockInvoke.mock.calls.some(([cmd]) => cmd === "set_gitlab_token"),
    ).toBe(false);
  });
});

describe("SettingsModal — AI tab", () => {
  it("turns AI analysis off at once", async () => {
    await renderTab("AI");
    fireEvent.click(screen.getByRole("switch", { name: "AI analysis" }));
    await waitFor(() =>
      expect(updates()).toEqual([
        { ...settings(), ai: { ...settings().ai, enabled: false } },
      ]),
    );
  });

  it("warns when the configured agent is auggie, with no run read yet", async () => {
    const warning =
      "Auggie can still use MCP tools from your own Auggie setup and read local secret files in the clone (.env files, keys); Argus cannot guarantee this run only reads.";
    const initialAi = useAiStore.getState();
    useAiStore.setState({ runs: {} });
    await renderTab("AI");
    expect(screen.queryByText(warning)).toBeNull();
    act(() =>
      useBackendSettingsStore.setState({
        settings: {
          ...settings(),
          ai: { ...settings().ai, adapter: "auggie" },
        },
      }),
    );
    expect(screen.getByText(warning)).toBeInTheDocument();
    useAiStore.setState(initialAi, true);
  });

  it("keeps the agent binary when it turns AI analysis off", async () => {
    const stored = settings({
      ai: { ...settings().ai, binary: "/opt/fixture/bin/claude" },
    });
    backend(stored, false);
    await renderTab("AI");
    fireEvent.click(screen.getByRole("switch", { name: "AI analysis" }));
    await waitFor(() =>
      expect(updates()).toEqual([
        { ...stored, ai: { ...stored.ai, enabled: false } },
      ]),
    );
    expect(updates()[0].ai.binary).toBe("/opt/fixture/bin/claude");
  });

  it("adds a clone picked with the dialog, keyed host/repo", async () => {
    await renderTab("AI");
    expect(screen.getByText("No clones set.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Add clone" }));
    fireEvent.change(screen.getByLabelText("Repository"), {
      target: { value: "https://github.com/acme-fixture/widgets.git" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Choose…" }));
    await waitFor(() =>
      expect(screen.getByLabelText("Folder")).toHaveValue("/src/widgets"),
    );
    expect(openDialog).toHaveBeenCalledWith(
      expect.objectContaining({ directory: true }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(updates()).toEqual([
        {
          ...settings(),
          clonePaths: { "github.com/acme-fixture/widgets": "/src/widgets" },
        },
      ]),
    );
    expect(
      await screen.findByText("github.com/acme-fixture/widgets"),
    ).toBeInTheDocument();
  });

  it("validates inline, edits and removes a row", async () => {
    backend(
      settings({
        clonePaths: {
          "github.com/acme-fixture/widgets": "/src/widgets",
          "gitlab.example.test/grp/proj": "/src/proj",
        },
      }),
      false,
    );
    await renderTab("AI");
    fireEvent.click(await screen.findByRole("button", { name: "Add clone" }));
    fireEvent.change(screen.getByLabelText("Repository"), {
      target: { value: "acme/widgets" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(screen.getByRole("alert")).toHaveTextContent("host/owner/name");
    fireEvent.change(screen.getByLabelText("Repository"), {
      target: { value: "github.com/acme-fixture/widgets" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(screen.getByRole("alert")).toHaveTextContent("already has a clone");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(updates()).toEqual([]);

    vi.mocked(openDialog).mockResolvedValueOnce("/work/proj");
    fireEvent.click(
      screen.getByRole("button", {
        name: "Edit the clone of gitlab.example.test/grp/proj",
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Choose…" }));
    await waitFor(() =>
      expect(screen.getByLabelText("Folder")).toHaveValue("/work/proj"),
    );
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(updates()[updates().length - 1]?.clonePaths).toEqual({
        "github.com/acme-fixture/widgets": "/src/widgets",
        "gitlab.example.test/grp/proj": "/work/proj",
      }),
    );

    fireEvent.click(
      await screen.findByRole("button", {
        name: "Remove the clone of github.com/acme-fixture/widgets",
      }),
    );
    await waitFor(() =>
      expect(updates()[updates().length - 1]?.clonePaths).toEqual({
        "gitlab.example.test/grp/proj": "/work/proj",
      }),
    );
  });
});
