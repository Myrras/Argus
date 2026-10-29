import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";

import type { CloneSuggestion } from "../ipc/types";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { useCloneSuggestStore } from "../state/cloneSuggestStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { useToastStore } from "../state/toastStore";
import { review, settings, accounts } from "../test/fixtures";
import { CloneSuggestionBar } from "./CloneSuggestionBar";

const mockInvoke = vi.mocked(invoke);
const r = review();
const REPO = `${r.host}/${r.repo}`;
const PATH = "/Users/fixture/code/widgets";

let found: CloneSuggestion | null;
let failing = false;

const calls = (name: string) =>
  mockInvoke.mock.calls.filter(([cmd]) => cmd === name);

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockImplementation(((cmd: string) => {
    switch (cmd) {
      case "suggest_clone":
        return Promise.resolve(found);
      case "use_clone_suggestion":
      case "ignore_clone_suggestion":
        return failing
          ? Promise.reject({ kind: "invalidInput", message: "not a clone" })
          : Promise.resolve(settings());
      case "get_settings":
        return Promise.resolve(settings());
      case "list_accounts":
        return Promise.resolve(accounts());
      default:
        return Promise.resolve(null);
    }
  }) as typeof invoke);
  found = { repo: REPO, path: PATH };
  failing = false;
  useCloneSuggestStore.setState({ byReview: {}, dismissed: {}, busy: {} });
  useToastStore.setState({ toasts: [] });
  useBackendSettingsStore.setState({ settings: settings() });
  useSettingsRequestStore.getState().clear();
});

describe("CloneSuggestionBar", () => {
  it("offers the clone it found, with its short path", async () => {
    render(<CloneSuggestionBar review={r} />);
    await screen.findByText(/Found a local clone of/);
    expect(screen.getByText(REPO)).toBeInTheDocument();
    expect(screen.getByText("~/code/widgets").title).toBe(PATH);
    expect(screen.getByRole("button", { name: "Use it" })).toBeInTheDocument();
  });

  it("opens Settings → Repositories on the repository, and writes nothing", async () => {
    render(<CloneSuggestionBar review={r} />);
    fireEvent.click(await screen.findByRole("button", { name: "Set up…" }));
    expect(useSettingsRequestStore.getState()).toMatchObject({
      request: "repositories",
      repo: REPO,
    });
    expect(calls("use_clone_suggestion")).toHaveLength(0);
    expect(calls("ignore_clone_suggestion")).toHaveLength(0);
    expect(calls("update_settings")).toHaveLength(0);
  });

  it("shows nothing when a clone is set, and does not even look", async () => {
    useBackendSettingsStore.setState({
      settings: settings({ clonePaths: { [REPO]: PATH } }),
    });
    render(<CloneSuggestionBar review={r} />);
    await waitFor(() => expect(calls("suggest_clone")).toHaveLength(0));
    expect(screen.queryByText(/Found a local clone/)).toBeNull();
  });

  it("shows nothing when nothing was found", async () => {
    found = null;
    render(<CloneSuggestionBar review={r} />);
    await waitFor(() => expect(calls("suggest_clone")).toHaveLength(1));
    expect(screen.queryByText(/Found a local clone/)).toBeNull();
  });

  it("stores the clone on 'Use it' and re-reads the settings", async () => {
    render(<CloneSuggestionBar review={r} />);
    fireEvent.click(await screen.findByRole("button", { name: "Use it" }));
    await waitFor(() => expect(calls("use_clone_suggestion")).toHaveLength(1));
    expect(calls("use_clone_suggestion")[0][1]).toEqual({
      reviewId: r.id,
      path: PATH,
    });
    await waitFor(() => expect(calls("get_settings")).toHaveLength(1));
    expect(screen.queryByText(/Found a local clone/)).toBeNull();
  });

  it("remembers 'No thanks' through the backend", async () => {
    render(<CloneSuggestionBar review={r} />);
    fireEvent.click(await screen.findByRole("button", { name: "No thanks" }));
    await waitFor(() =>
      expect(calls("ignore_clone_suggestion")).toHaveLength(1),
    );
    expect(calls("ignore_clone_suggestion")[0][1]).toEqual({
      reviewId: r.id,
    });
    expect(screen.queryByText(/Found a local clone/)).toBeNull();
  });

  it("toasts the core's text when the folder is refused", async () => {
    failing = true;
    render(<CloneSuggestionBar review={r} />);
    fireEvent.click(await screen.findByRole("button", { name: "Use it" }));
    await waitFor(() =>
      expect(useToastStore.getState().toasts[0]?.detail).toContain(
        "not a clone",
      ),
    );
    expect(screen.getByText(/Found a local clone/)).toBeInTheDocument();
  });

  it("the × hides it for this session only, writing nothing", async () => {
    render(<CloneSuggestionBar review={r} />);
    fireEvent.click(
      await screen.findByRole("button", {
        name: "Hide this suggestion for now",
      }),
    );
    expect(screen.queryByText(/Found a local clone/)).toBeNull();
    expect(useCloneSuggestStore.getState().dismissed[REPO]).toBe(true);
    expect(calls("ignore_clone_suggestion")).toHaveLength(0);
    expect(calls("use_clone_suggestion")).toHaveLength(0);
  });
});
