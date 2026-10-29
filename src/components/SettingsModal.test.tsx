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
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(() => Promise.resolve("/src/widgets")),
}));
vi.mock("@tauri-apps/plugin-log", () => ({
  error: vi.fn(() => Promise.resolve()),
  warn: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

import type { Account, Settings } from "../ipc/types";
import { useAiStore } from "../state/aiStore";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { useSettingsStore } from "../state/settingsStore";
import { useThemeStore } from "../state/themeStore";
import { useToastStore } from "../state/toastStore";
import { account, accounts, settings } from "../test/fixtures";
import { SettingsModal } from "./SettingsModal";

const mockInvoke = vi.mocked(invoke);
const initialBackend = useBackendSettingsStore.getState();
/**
 * A fake core: `update_settings` stores what it gets; `list_accounts`
 * answers `rows` with the stored choices applied, and with `recheck` (Check
 * again) the CLIs' answer now, `after` when given.
 */
function backend(
  initial: Settings,
  rows: Account[] = accounts(),
  after?: Account[],
) {
  let stored = initial;
  mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
    switch (cmd) {
      case "get_settings":
        return Promise.resolve(stored);
      case "update_settings":
        stored = (args as { settings: Settings }).settings;
        return Promise.resolve(stored);
      case "list_accounts": {
        const recheck = (args as { recheck: boolean }).recheck;
        const now = recheck && after ? after : rows;
        return Promise.resolve(
          now.map((a) => ({ ...a, ...stored.accounts[a.key] })),
        );
      }
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
  backend(settings());
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

  it("shows the disk cache, steps its limit and clears it", async () => {
    let stats = { entries: 1200, bytes: 300 * 1024 * 1024 };
    const inner = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "cache_stats") return Promise.resolve(stats);
      if (cmd === "clear_cache") {
        stats = { entries: 0, bytes: 0 };
        return Promise.resolve(stats);
      }
      return inner(cmd, args as never);
    });
    render(<SettingsModal open onClose={() => undefined} />);
    expect(
      await screen.findByText("300 MB in 1200 entries"),
    ).toBeInTheDocument();
    // No limit stored yet: the default.
    await waitFor(() =>
      expect(screen.getByLabelText("Cache limit")).toHaveTextContent("512 MB"),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Increase cache limit" }),
    );
    await waitFor(() =>
      expect(screen.getByLabelText("Cache limit")).toHaveTextContent("1 GB"),
    );
    expect(updates()).toEqual([{ ...settings(), cacheLimitMb: 1024 }]);
    fireEvent.click(screen.getByRole("button", { name: "Clear cache" }));
    expect(await screen.findByText("0.0 MB in 0 entries")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Clear cache" })).toBeDisabled();
  });

  it("opens on the tab a settings request asks for", () => {
    useSettingsRequestStore.getState().open("accounts");
    render(<SettingsModal open onClose={() => undefined} />);
    expect(screen.getByRole("tab", { name: "Accounts" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("has one Accounts tab in place of GitHub and GitLab", () => {
    render(<SettingsModal open onClose={() => undefined} />);
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual([
      "General",
      "Accounts",
      "AI",
      "Repositories",
      "Editors",
      "Scripts",
      "Shortcuts",
    ]);
  });
});

describe("SettingsModal — Accounts tab", () => {
  const GITLAB = "gitlab.example.test";
  const LOGIN = `glab auth login --hostname ${GITLAB}`;
  /** Every command the dialog sent, by name. */
  const sent = () => mockInvoke.mock.calls.map(([cmd]) => cmd);
  const card = (name: string) => screen.getByRole("group", { name });

  it("lists every host of gh and glab, logged in, never asking for a token", async () => {
    await renderTab("Accounts");
    expect(
      screen.getByText(/never sees or stores a token/),
    ).toBeInTheDocument();
    const gh = await screen.findByRole("group", { name: "GitHub github.com" });
    expect(within(gh).getByText("Logged in as rev-fixture")).toBeVisible();
    expect(
      within(gh).getByRole("switch", { name: "Use GitHub at github.com" }),
    ).toHaveAttribute("aria-checked", "true");
    const gl = card(`GitLab ${GITLAB}`);
    expect(within(gl).getByText("Logged in as rev-fixture")).toBeVisible();
    // Logged in: no login command on the card.
    expect(within(gl).queryByText(LOGIN)).not.toBeInTheDocument();
    // gh and glab hold the logins: no token field at all.
    expect(screen.queryByLabelText(/token/i)).not.toBeInTheDocument();
    expect(document.querySelector('input[type="password"]')).toBeNull();
    expect(sent().every((c) => !c.includes("token"))).toBe(true);
  });

  it("switches a host off at once", async () => {
    await renderTab("Accounts");
    const toggle = await screen.findByRole("switch", {
      name: `Use GitLab at ${GITLAB}`,
    });
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(toggle).toHaveAttribute("aria-checked", "false"),
    );
    expect(updates()).toEqual([
      {
        ...settings(),
        accounts: { [`gitlab:${GITLAB}`]: { enabled: false, account: null } },
      },
    ]);
    expect(within(card(`GitLab ${GITLAB}`)).getByText("Off")).toBeVisible();
  });

  it("shows a host that is not logged in off, with its login command", async () => {
    backend(settings(), [
      account({
        provider: "gitlab",
        host: "gitlab.com",
        key: "gitlab:gitlab.com",
        enabled: false,
        state: { kind: "notLoggedIn", message: null },
        loginCommand: "glab auth login --hostname gitlab.com",
      }),
    ]);
    await renderTab("Accounts");
    const toggle = await screen.findByRole("switch", {
      name: "Use GitLab at gitlab.com",
    });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    expect(
      within(card("GitLab gitlab.com")).getByText(
        "glab auth login --hostname gitlab.com",
      ),
    ).toBeVisible();
  });

  it("picks one of several gh accounts on a host", async () => {
    backend(settings(), [
      account({
        accounts: ["rev-fixture", "work-fixture"],
        active: "rev-fixture",
      }),
    ]);
    await renderTab("Accounts");
    const picker = await screen.findByLabelText("gh account");
    expect(picker).toHaveValue("");
    expect(
      screen.getByRole("option", { name: "gh's active account (rev-fixture)" }),
    ).toBeInTheDocument();
    fireEvent.change(picker, { target: { value: "work-fixture" } });
    await waitFor(() =>
      expect(updates()).toEqual([
        {
          ...settings(),
          accounts: {
            "github:github.com": { enabled: true, account: "work-fixture" },
          },
        },
      ]),
    );
    await waitFor(() => expect(picker).toHaveValue("work-fixture"));
  });

  it("has no picker for a host with one gh account", async () => {
    await renderTab("Accounts");
    await screen.findByRole("group", { name: "GitHub github.com" });
    expect(screen.queryByLabelText("gh account")).not.toBeInTheDocument();
  });

  it("gives the exact login command for a host not logged in, and checks again", async () => {
    const loggedOut = account({
      provider: "gitlab",
      host: GITLAB,
      user: null,
      state: {
        kind: "notLoggedIn",
        message: `glab has no working login for ${GITLAB}.`,
      },
    });
    backend(settings(), [account(), loggedOut], accounts());
    await renderTab("Accounts");
    const gl = await screen.findByRole("group", { name: `GitLab ${GITLAB}` });
    expect(within(gl).getByText("Not logged in")).toBeVisible();
    expect(
      within(gl).getByText(`glab has no working login for ${GITLAB}.`),
    ).toBeVisible();
    // In a code block (highlighted once loaded) with its own copy button.
    expect(gl).toHaveTextContent(LOGIN);
    expect(within(gl).getByRole("button", { name: "Copy code" })).toBeVisible();

    mockInvoke.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    // The forced check: the core asks the CLIs even with a current answer.
    await waitFor(() =>
      expect(mockInvoke.mock.calls).toEqual([
        ["list_accounts", { recheck: true }],
      ]),
    );
    await waitFor(() =>
      expect(
        within(card(`GitLab ${GITLAB}`)).getByText("Logged in as rev-fixture"),
      ).toBeVisible(),
    );
  });

  it("says to install a missing CLI", async () => {
    backend(settings(), [
      account(),
      account({
        provider: "gitlab",
        key: "gitlab:",
        host: "",
        user: null,
        state: {
          kind: "cliMissing",
          hint: "Install the GitLab CLI (brew install glab), then run glab auth login.",
        },
        loginCommand: "glab auth login",
      }),
    ]);
    await renderTab("Accounts");
    const gl = await screen.findByRole("group", { name: "GitLab GitLab" });
    expect(within(gl).getByText("glab not found")).toBeVisible();
    expect(within(gl).getByText(/brew install glab/)).toBeVisible();
    expect(gl).toHaveTextContent("glab auth login");
    // Nothing to switch without a host.
    expect(within(gl).queryByRole("switch")).not.toBeInTheDocument();
  });

  it("tells a host it could not check apart from a missing login", async () => {
    backend(settings(), [
      account({
        provider: "gitlab",
        host: GITLAB,
        state: { kind: "error", message: `glab could not reach ${GITLAB}.` },
      }),
    ]);
    await renderTab("Accounts");
    const gl = await screen.findByRole("group", { name: `GitLab ${GITLAB}` });
    expect(within(gl).getByText("Couldn't check")).toBeVisible();
    expect(within(gl).getByText(/could not reach/)).toBeVisible();
    expect(within(gl).queryByText(LOGIN)).not.toBeInTheDocument();
  });

  it("fills a new host into both login commands", async () => {
    await renderTab("Accounts");
    const add = screen.getByRole("region", { name: "Add an account" });
    expect(add).toHaveTextContent("gh auth login");
    expect(add).not.toHaveTextContent("--hostname");
    fireEvent.change(within(add).getByLabelText("Host (optional)"), {
      target: { value: " ghe.example.test " },
    });
    await waitFor(() => {
      expect(add).toHaveTextContent(
        "gh auth login --hostname ghe.example.test",
      );
      expect(add).toHaveTextContent(
        "glab auth login --hostname ghe.example.test",
      );
    });
  });

  it("opens on an account a request asks for, highlighted", async () => {
    useSettingsRequestStore.getState().open("accounts", `gitlab:${GITLAB}`);
    render(<SettingsModal open onClose={() => undefined} />);
    const gl = await screen.findByRole("group", { name: `GitLab ${GITLAB}` });
    await waitFor(() => expect(gl).toHaveAttribute("data-highlighted", "true"));
    expect(card("GitHub github.com")).not.toHaveAttribute("data-highlighted");
  });

  it("says so when there is no account yet", async () => {
    backend(settings(), []);
    await renderTab("Accounts");
    expect(await screen.findByText(/No accounts yet/)).toBeVisible();
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
      "Argus turns off Auggie's MCP and tools beyond reading, but Auggie has no path rules, so its read tool can open any file on your machine, and your own Auggie hooks still run; Argus cannot guarantee this run only reads.";
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
    backend(stored);
    await renderTab("AI");
    fireEvent.click(screen.getByRole("switch", { name: "AI analysis" }));
    await waitFor(() =>
      expect(updates()).toEqual([
        { ...stored, ai: { ...stored.ai, enabled: false } },
      ]),
    );
    expect(updates()[0].ai.binary).toBe("/opt/fixture/bin/claude");
  });

  it("keeps the pass itself; clones and review skills moved to Repositories", async () => {
    backend(
      settings({ clonePaths: { "github.com/acme-fixture/widgets": "/src/w" } }),
    );
    await renderTab("AI");
    expect(screen.queryByText("Local clones")).toBeNull();
    expect(screen.queryByText("Review skill")).toBeNull();
    expect(screen.queryByRole("button", { name: "Add clone" })).toBeNull();
    expect(screen.queryByText("github.com/acme-fixture/widgets")).toBeNull();
    fireEvent.click(
      screen.getByRole("button", { name: "Settings → Repositories" }),
    );
    expect(screen.getByRole("tab", { name: "Repositories" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("saves the default review language at once, another one with its own Save", async () => {
    await renderTab("AI");
    const select = screen.getByRole("combobox", {
      name: "Default review language",
    });
    // Auto when the settings have none (an older file); no "Default" here.
    expect(select).toHaveValue("auto");
    expect(
      screen.queryByRole("option", { name: /^Default/ }),
    ).not.toBeInTheDocument();
    fireEvent.change(select, { target: { value: "dutch" } });
    await waitFor(() =>
      expect(updates()[0]?.ai.reviewLanguage).toEqual({ kind: "dutch" }),
    );
    // Other…: nothing saved until the name is.
    fireEvent.change(select, { target: { value: "other" } });
    const name = screen.getByRole("textbox", {
      name: "Default review language: language name",
    });
    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();
    expect(updates()).toHaveLength(1);
    fireEvent.change(name, { target: { value: "  Deutsch " } });
    fireEvent.click(save);
    await waitFor(() =>
      expect(updates()[1]?.ai.reviewLanguage).toEqual({
        kind: "other",
        name: "Deutsch",
      }),
    );
    // The rest of the AI settings stay as they were.
    expect(updates()[1]?.ai.enabled).toBe(true);
    expect(updates()[1]?.ai.adapter).toBe("claude");
  });
});

describe("SettingsModal — Editors tab", () => {
  it("sets an editor per extension with the app picker, as in Iris", async () => {
    await renderTab("Editors");
    expect(screen.getByText("System default")).toBeInTheDocument();
    expect(
      screen.getByText("No editors per extension set."),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Add editor" }));
    fireEvent.change(screen.getByLabelText("Extension"), {
      target: { value: "tar.gz" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(screen.getByRole("alert")).toHaveTextContent("single extension");
    fireEvent.change(screen.getByLabelText("Extension"), {
      target: { value: " .MD " },
    });
    vi.mocked(openDialog).mockResolvedValueOnce("/Applications/Typora.app");
    fireEvent.click(screen.getByRole("button", { name: "Choose…" }));
    await waitFor(() =>
      expect(screen.getByLabelText("Application")).toHaveValue(
        "/Applications/Typora.app",
      ),
    );
    expect(openDialog).toHaveBeenCalledWith(
      expect.objectContaining({ directory: false }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(updates()).toEqual([
        {
          ...settings(),
          editorOverrides: { md: "/Applications/Typora.app" },
        },
      ]),
    );
    expect(await screen.findByText("Typora")).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Remove the .md editor" }),
    );
    await waitFor(() =>
      expect(updates()[updates().length - 1]?.editorOverrides).toEqual({}),
    );
  });

  it("switches opening the repository with the file", async () => {
    await renderTab("Editors");
    const toggle = screen.getByRole("switch", {
      name: "Open the repository with the file",
    });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(updates()[updates().length - 1]?.openWithRepository).toBe(false),
    );
  });

  it("chooses and resets the default editor", async () => {
    await renderTab("Editors");
    vi.mocked(openDialog).mockResolvedValueOnce(
      "/Applications/Visual Studio Code.app",
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Choose the default editor" }),
    );
    expect(await screen.findByText("Visual Studio Code")).toBeInTheDocument();
    expect(updates()[0]?.defaultEditor).toBe(
      "/Applications/Visual Studio Code.app",
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Use the system default" }),
    );
    await waitFor(() =>
      expect(updates()[updates().length - 1]?.defaultEditor).toBeNull(),
    );
    expect(await screen.findByText("System default")).toBeInTheDocument();
  });
});

describe("SettingsModal — layout", () => {
  it("scrolls only the tab pane, the dialog and its tabs stay put", () => {
    render(<SettingsModal open onClose={() => undefined} />);
    const pane = screen.getByTestId("settings-pane");
    const dialog = screen.getByRole("dialog", { name: "Settings" });
    expect(pane).toHaveAttribute("role", "tabpanel");
    expect(pane.className).toMatch(/pane/);
    // The tab strip is a sibling of the pane, not inside what scrolls.
    expect(pane).not.toContainElement(screen.getByRole("tablist"));
    expect(dialog).toContainElement(pane);
  });

  it("starts a new tab at the top of the pane", () => {
    render(<SettingsModal open onClose={() => undefined} />);
    const pane = screen.getByTestId("settings-pane");
    pane.scrollTop = 120;
    fireEvent.click(screen.getByRole("tab", { name: "Shortcuts" }));
    expect(pane.scrollTop).toBe(0);
  });
});
