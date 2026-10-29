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
  open: vi.fn(() => Promise.resolve("/Users/fixture/code/widgets")),
}));
vi.mock("@tauri-apps/plugin-log", () => ({
  error: vi.fn(() => Promise.resolve()),
  warn: vi.fn(() => Promise.resolve()),
}));
const events = vi.hoisted(() => {
  const handlers = new Map<string, ((e: { payload: unknown }) => void)[]>();
  return {
    listen: (name: string, cb: (e: { payload: unknown }) => void) => {
      handlers.set(name, [...(handlers.get(name) ?? []), cb]);
      return Promise.resolve(() =>
        handlers.set(
          name,
          (handlers.get(name) ?? []).filter((h) => h !== cb),
        ),
      );
    },
    emit: (name: string, payload: unknown) =>
      (handlers.get(name) ?? []).forEach((h) => h({ payload })),
    clear: () => handlers.clear(),
  };
});
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) =>
    events.listen(name, cb),
  ),
}));

import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

import type {
  CloneState,
  ProviderId,
  RepoSetup,
  ReviewSkillList,
  Settings,
} from "../ipc/types";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { useRepositoriesStore } from "../state/repositoriesStore";
import { useReviewSkillsStore } from "../state/reviewSkillsStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { useToastStore } from "../state/toastStore";
import { settings, accounts } from "../test/fixtures";
import { SettingsModal } from "./SettingsModal";

const mockInvoke = vi.mocked(invoke);
const initialBackend = useBackendSettingsStore.getState();
const WIDGETS = "github.com/acme-fixture/widgets";
const API = "gitlab.example.test/acme-fixture/api";
const CLONE = "/Users/fixture/code/widgets";

const skillList: ReviewSkillList = {
  skills: [{ name: "my-review", description: "Mine.", source: "user" }],
  autoPick: null,
  projectListed: true,
};

/** The repositories the fake core has seen reviews of. */
let seen: { key: string; provider: ProviderId; openReviews: number }[];
/** Folders the fake core finds missing. */
let missing: Set<string>;

/** A fake core: settings stored as saved; the list built like the core's. */
function backend(initial: Settings) {
  let stored = initial;
  const list = (): RepoSetup[] => {
    const keys = new Set([
      ...seen.map((r) => r.key),
      ...Object.keys(stored.clonePaths),
      ...Object.keys(stored.reviewSkills ?? {}),
      ...Object.keys(stored.reviewLanguages ?? {}),
    ]);
    return [...keys]
      .map((key): RepoSetup => {
        const r = seen.find((x) => x.key === key);
        const path = stored.clonePaths[key] ?? null;
        const clone: CloneState = !path
          ? "notSet"
          : missing.has(path)
            ? "missing"
            : "found";
        const [host, ...rest] = key.split("/");
        return {
          key,
          host,
          repo: rest.join("/"),
          provider: r?.provider ?? null,
          openReviews: r?.openReviews ?? 0,
          clonePath: path,
          clone,
          reviewSkill: stored.reviewSkills?.[key] ?? { kind: "auto" },
          reviewLanguage: stored.reviewLanguages?.[key] ?? null,
        };
      })
      .sort(
        (a, b) =>
          Number(b.openReviews > 0) - Number(a.openReviews > 0) ||
          a.key.localeCompare(b.key),
      );
  };
  mockInvoke.mockImplementation(((cmd: string, args?: unknown) => {
    switch (cmd) {
      case "get_settings":
        return Promise.resolve(stored);
      case "update_settings":
        stored = (args as { settings: Settings }).settings;
        return Promise.resolve(stored);
      case "list_repositories":
        return Promise.resolve(list());
      case "list_review_skills":
        return Promise.resolve(skillList);
      case "list_accounts":
        return Promise.resolve(accounts());
      default:
        return Promise.resolve(undefined);
    }
  }) as typeof invoke);
}

const updates = () =>
  mockInvoke.mock.calls
    .filter(([cmd]) => cmd === "update_settings")
    .map(([, args]) => (args as { settings: Settings }).settings);
const last = () => updates()[updates().length - 1];
const calls = (name: string) =>
  mockInvoke.mock.calls.filter(([cmd]) => cmd === name);

async function renderTab() {
  render(<SettingsModal open onClose={() => undefined} />);
  fireEvent.click(screen.getByRole("tab", { name: "Repositories" }));
  await waitFor(() =>
    expect(useRepositoriesStore.getState().repos).not.toBeNull(),
  );
}

const card = (key: string) => screen.getByRole("group", { name: key });
const cards = () =>
  screen
    .queryAllByRole("group")
    .filter((g) => g.dataset.slot === "repo-card")
    .map((g) => g.getAttribute("aria-label"));

beforeEach(() => {
  vi.clearAllMocks();
  events.clear();
  seen = [
    { key: WIDGETS, provider: "github", openReviews: 2 },
    { key: API, provider: "gitlab", openReviews: 0 },
  ];
  missing = new Set();
  useBackendSettingsStore.setState({ ...initialBackend }, true);
  useRepositoriesStore.setState({ repos: null, error: null });
  useReviewSkillsStore.setState({ lists: {}, loading: {}, errors: {} });
  useSettingsRequestStore.getState().clear();
  useToastStore.setState({ toasts: [] });
  backend(settings());
});

describe("Settings → Repositories", () => {
  it("sits after AI and lists one card per repository, open ones first", async () => {
    await renderTab();
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual([
      "General",
      "Accounts",
      "AI",
      "Repositories",
      "Editors",
      "Scripts",
      "Shortcuts",
    ]);
    await waitFor(() => expect(cards()).toEqual([WIDGETS, API]));
    const widgets = card(WIDGETS);
    expect(within(widgets).getByText("GitHub")).toBeInTheDocument();
    expect(within(widgets).getByText("2 open")).toBeInTheDocument();
    expect(within(widgets).getByText("Not set up")).toBeInTheDocument();
    expect(within(widgets).getByText("Not set")).toBeInTheDocument();
    expect(within(card(API)).getByText("GitLab")).toBeInTheDocument();
    expect(within(card(API)).queryByText(/open$/)).toBeNull();
    // Each card has its three settings; no filter for a short list.
    for (const label of ["Local clone", "Review skill", "Review language"]) {
      expect(within(widgets).getByText(label)).toBeInTheDocument();
    }
    expect(screen.queryByRole("searchbox")).toBeNull();
  });

  it("adds a repository a refresh found, while the tab shows", async () => {
    await renderTab();
    await waitFor(() => expect(cards()).toEqual([WIDGETS, API]));
    seen.push({
      key: "github.com/acme-fixture/gadgets",
      provider: "github",
      openReviews: 1,
    });
    act(() =>
      events.emit("reviews:refreshed", {
        changed: [],
        errors: [],
        refreshedAt: "2026-10-23T10:00:00Z",
      }),
    );
    await waitFor(() =>
      expect(cards()).toEqual([
        "github.com/acme-fixture/gadgets",
        WIDGETS,
        API,
      ]),
    );
    expect(
      within(card("github.com/acme-fixture/gadgets")).getByText("Not set up"),
    ).toBeInTheDocument();
  });

  it("chooses, shows and clears a clone at once", async () => {
    await renderTab();
    const widgets = await waitFor(() => card(WIDGETS));
    fireEvent.click(
      within(widgets).getByRole("button", {
        name: `Choose the local clone of ${WIDGETS}`,
      }),
    );
    await waitFor(() =>
      expect(last()?.clonePaths).toEqual({ [WIDGETS]: CLONE }),
    );
    expect(openDialog).toHaveBeenCalledWith(
      expect.objectContaining({ directory: true }),
    );
    await waitFor(() =>
      expect(within(card(WIDGETS)).getByText("Found")).toBeInTheDocument(),
    );
    expect(within(card(WIDGETS)).getByText("~/code/widgets").title).toBe(CLONE);
    expect(within(card(WIDGETS)).queryByText("Not set up")).toBeNull();
    fireEvent.click(
      within(card(WIDGETS)).getByRole("button", {
        name: /^Show the clone of github.com\/acme-fixture\/widgets in/,
      }),
    );
    await waitFor(() => expect(calls("reveal_repo_clone")).toHaveLength(1));
    expect(calls("reveal_repo_clone")[0][1]).toEqual({ repo: WIDGETS });
    fireEvent.click(
      within(card(WIDGETS)).getByRole("button", {
        name: `Clear the local clone of ${WIDGETS}`,
      }),
    );
    await waitFor(() => expect(last()?.clonePaths).toEqual({}));
  });

  it("says when a clone's folder is gone, and offers no Show", async () => {
    missing.add("/gone/widgets");
    backend(settings({ clonePaths: { [WIDGETS]: "/gone/widgets" } }));
    await renderTab();
    await waitFor(() =>
      expect(
        within(card(WIDGETS)).getByText("Folder not found"),
      ).toBeInTheDocument(),
    );
    expect(
      within(card(WIDGETS)).queryByRole("button", { name: /^Show the clone/ }),
    ).toBeNull();
  });

  it("saves the review skill per repository", async () => {
    await renderTab();
    const widgets = await waitFor(() => card(WIDGETS));
    fireEvent.click(
      within(widgets).getByRole("button", {
        name: `Choose the review skill of ${WIDGETS}`,
      }),
    );
    fireEvent.click(
      await within(widgets).findByRole("radio", { name: "my-review, yours" }),
    );
    await waitFor(() =>
      expect(last()?.reviewSkills).toEqual({
        [WIDGETS]: { kind: "skill", source: "user", name: "my-review" },
      }),
    );
  });

  it("saves the review language: Default follows the AI tab, Other… with its own Save", async () => {
    backend(
      settings({
        ai: { ...settings().ai, reviewLanguage: { kind: "english" } },
      }),
    );
    await renderTab();
    const select = await screen.findByRole("combobox", {
      name: `Review language of ${WIDGETS}`,
    });
    expect(select).toHaveValue("default");
    expect(
      within(select).getByRole("option", { name: "Default (English)" }),
    ).toBeInTheDocument();
    fireEvent.change(select, { target: { value: "auto" } });
    await waitFor(() =>
      expect(last()?.reviewLanguages).toEqual({ [WIDGETS]: { kind: "auto" } }),
    );
    fireEvent.change(select, { target: { value: "dutch" } });
    await waitFor(() =>
      expect(last()?.reviewLanguages).toEqual({
        [WIDGETS]: { kind: "dutch" },
      }),
    );
    fireEvent.change(select, { target: { value: "other" } });
    expect(updates()).toHaveLength(2);
    fireEvent.change(
      screen.getByRole("textbox", {
        name: `Review language of ${WIDGETS}: language name`,
      }),
      { target: { value: "Deutsch" } },
    );
    fireEvent.click(
      within(card(WIDGETS)).getByRole("button", { name: "Save" }),
    );
    await waitFor(() =>
      expect(last()?.reviewLanguages).toEqual({
        [WIDGETS]: { kind: "other", name: "Deutsch" },
      }),
    );
    fireEvent.change(select, { target: { value: "default" } });
    await waitFor(() => expect(last()?.reviewLanguages).toEqual({}));
    // The global default was never touched.
    expect(last()?.ai.reviewLanguage).toEqual({ kind: "english" });
  });

  it("filters a long list by name", async () => {
    seen = Array.from({ length: 8 }, (_, i) => ({
      key: `github.com/acme-fixture/repo-${i}`,
      provider: "github" as const,
      openReviews: 0,
    }));
    await renderTab();
    await waitFor(() => expect(cards()).toHaveLength(8));
    const filter = screen.getByRole("searchbox", {
      name: "Filter repositories",
    });
    fireEvent.change(filter, { target: { value: "REPO-3" } });
    expect(cards()).toEqual(["github.com/acme-fixture/repo-3"]);
    fireEvent.change(filter, { target: { value: "nothing" } });
    expect(cards()).toEqual([]);
    expect(screen.getByText(/No repository matches/)).toBeInTheDocument();
  });

  it("adds a repository by hand, checked inline", async () => {
    seen = [];
    await renderTab();
    expect(await screen.findByText(/No repositories yet/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Add repository" }));
    fireEvent.change(screen.getByLabelText("Repository"), {
      target: { value: "acme/widgets" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(screen.getByRole("alert")).toHaveTextContent("host/owner/name");
    fireEvent.change(screen.getByLabelText("Repository"), {
      target: { value: "https://github.com/acme-fixture/widgets.git" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Choose…" }));
    await waitFor(() =>
      expect(screen.getByLabelText("Local clone")).toHaveValue(CLONE),
    );
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(last()?.clonePaths).toEqual({ [WIDGETS]: CLONE }),
    );
    await waitFor(() => expect(cards()).toEqual([WIDGETS]));
  });

  it("opens on a requested repository and highlights it", async () => {
    seen = Array.from({ length: 8 }, (_, i) => ({
      key: `github.com/acme-fixture/repo-${i}`,
      provider: "github" as const,
      openReviews: 0,
    }));
    useSettingsRequestStore
      .getState()
      .open("repositories", "github.com/acme-fixture/repo-5");
    render(<SettingsModal open onClose={() => undefined} />);
    expect(screen.getByRole("tab", { name: "Repositories" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await waitFor(() => expect(cards()).toHaveLength(8));
    expect(card("github.com/acme-fixture/repo-5")).toHaveAttribute(
      "data-highlighted",
      "true",
    );
    expect(card("github.com/acme-fixture/repo-1")).not.toHaveAttribute(
      "data-highlighted",
    );
    // A filter that hid it is cleared by the next request.
    fireEvent.change(screen.getByRole("searchbox"), {
      target: { value: "repo-1" },
    });
    expect(cards()).toEqual(["github.com/acme-fixture/repo-1"]);
    act(() =>
      useSettingsRequestStore
        .getState()
        .open("repositories", "github.com/acme-fixture/repo-5"),
    );
    await waitFor(() => expect(cards()).toHaveLength(8));
    expect(card("github.com/acme-fixture/repo-5")).toHaveAttribute(
      "data-highlighted",
      "true",
    );
  });

  it("writes nothing on its own: only the controls save", async () => {
    await renderTab();
    await waitFor(() => expect(cards()).toEqual([WIDGETS, API]));
    act(() =>
      events.emit("reviews:refreshed", {
        changed: [],
        errors: [],
        refreshedAt: "2026-10-23T10:00:00Z",
      }),
    );
    await waitFor(() =>
      expect(calls("list_repositories").length).toBeGreaterThan(1),
    );
    expect(updates()).toEqual([]);
  });
});
