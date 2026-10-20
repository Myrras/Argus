import { beforeEach, describe, expect, it, vi } from "vitest";

import { fakeEvents, settings, status } from "../test/fixtures";

const events = fakeEvents();
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) =>
    events.listen(name, cb),
  ),
}));
vi.mock("./toastStore", () => ({ toastError: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";

import type { ProviderStatus, Settings } from "../ipc/types";
import {
  subscribeBackendSettingsEvents,
  useBackendSettingsStore,
} from "./backendSettingsStore";
import { toastError } from "./toastStore";

const mockInvoke = vi.mocked(invoke);
const initial = useBackendSettingsStore.getState();
const TOKEN = "glpat-fixture-not-a-real-token";

function backend(st: ProviderStatus, tokenPresent: boolean) {
  mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
    switch (cmd) {
      case "get_settings":
        return Promise.resolve(settings());
      case "update_settings":
        return Promise.resolve((args as { settings: Settings }).settings);
      case "provider_status":
        return Promise.resolve(st);
      case "gitlab_token_present":
        return Promise.resolve(tokenPresent);
      default:
        return Promise.resolve(undefined);
    }
  });
}

describe("backendSettingsStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useBackendSettingsStore.setState({ ...initial }, true);
  });

  it("loads settings, provider status and token presence", async () => {
    backend(status(), true);
    await useBackendSettingsStore.getState().load();
    expect(useBackendSettingsStore.getState()).toMatchObject({
      settings: settings(),
      status: status(),
      tokenPresent: true,
    });
  });

  it("saves a patch over the stored settings, then re-checks the providers", async () => {
    backend(status(), false);
    await useBackendSettingsStore.getState().load();
    mockInvoke.mockClear();
    const ok = await useBackendSettingsStore
      .getState()
      .save({ pollIntervalSecs: 300 });
    expect(ok).toBe(true);
    expect(mockInvoke).toHaveBeenCalledWith("update_settings", {
      settings: { ...settings(), pollIntervalSecs: 300 },
    });
    expect(mockInvoke).toHaveBeenCalledWith("provider_status");
    expect(useBackendSettingsStore.getState().settings?.pollIntervalSecs).toBe(
      300,
    );
  });

  it("reports a rejected save and re-reads the stored value", async () => {
    backend(status(), false);
    await useBackendSettingsStore.getState().load();
    mockInvoke.mockImplementationOnce(() =>
      Promise.reject({
        kind: "invalidInput",
        message: "the GitLab URL must start with https:// or http://",
      }),
    );
    const ok = await useBackendSettingsStore
      .getState()
      .save({ gitlab: { baseUrl: "gitlab.example.test" } });
    expect(ok).toBe(false);
    expect(toastError).toHaveBeenCalledWith(
      "Could not save the settings",
      "the GitLab URL must start with https:// or http://",
    );
    expect(useBackendSettingsStore.getState().settings).toEqual(settings());
  });

  it("never puts the token in update_settings", async () => {
    backend(status(), true);
    await useBackendSettingsStore.getState().load();
    await useBackendSettingsStore.getState().save({
      gitlab: { baseUrl: "https://gitlab.example.test" },
    });
    const ok = await useBackendSettingsStore.getState().setGitlabToken(TOKEN);
    expect(ok).toBe(true);
    const updates = mockInvoke.mock.calls.filter(
      ([c]) => c === "update_settings",
    );
    expect(updates).toHaveLength(1);
    expect(JSON.stringify(updates)).not.toContain(TOKEN);
    expect(mockInvoke).toHaveBeenCalledWith("set_gitlab_token", {
      token: TOKEN,
    });
    expect(useBackendSettingsStore.getState().tokenPresent).toBe(true);
  });

  it("notes when a provider's setup changed, as asked for", async () => {
    backend(status(), false);
    await useBackendSettingsStore.getState().load();
    await useBackendSettingsStore.getState().save({ pollIntervalSecs: 300 });
    expect(useBackendSettingsStore.getState().setupChanged).toEqual({});

    const before = Date.now();
    await useBackendSettingsStore.getState().save({
      gitlab: { baseUrl: "http://gitlab.vpn.test" },
    });
    const saved = useBackendSettingsStore.getState().setupChanged;
    expect(Object.keys(saved)).toEqual(["gitlab"]);
    expect(saved.gitlab).toBeGreaterThanOrEqual(before);

    // A token the keychain refused changes nothing.
    mockInvoke.mockRejectedValueOnce({ kind: "provider", message: "nope" });
    await useBackendSettingsStore.getState().setGitlabToken(TOKEN);
    expect(useBackendSettingsStore.getState().setupChanged).toEqual(saved);

    const asked = Date.now();
    await useBackendSettingsStore.getState().setGitlabToken(TOKEN);
    expect(
      useBackendSettingsStore.getState().setupChanged.gitlab,
    ).toBeGreaterThanOrEqual(asked);
  });

  it("re-checks the providers even when storing the token failed", async () => {
    // The keychain may hold the new token although the call failed (a
    // leftover entry that would not go): what is stored is re-read.
    backend(status(), false);
    await useBackendSettingsStore.getState().load();
    mockInvoke.mockClear();
    mockInvoke.mockRejectedValueOnce({ kind: "provider", message: "nope" });
    const ok = await useBackendSettingsStore.getState().setGitlabToken(TOKEN);
    expect(ok).toBe(false);
    expect(toastError).toHaveBeenCalledWith(
      "Could not store the token",
      "nope",
    );
    expect(mockInvoke).toHaveBeenCalledWith("provider_status");
    expect(mockInvoke).toHaveBeenCalledWith("gitlab_token_present");
  });

  it("stores or removes the token only after the saves before it", async () => {
    backend(status(), true);
    await useBackendSettingsStore.getState().load();
    const answer = mockInvoke.getMockImplementation()!;
    const order: string[] = [];
    let release = () => undefined as void;
    mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "set_gitlab_token" || cmd === "update_settings") {
        order.push(cmd);
      }
      if (cmd !== "update_settings") return answer(cmd, args as never);
      const body = (args as { settings: Settings }).settings;
      return new Promise((r) => {
        release = () => r(body);
      });
    });
    const store = useBackendSettingsStore.getState();
    const saved = store.save({
      gitlab: { baseUrl: "https://gl.example.test" },
    });
    const removed = store.setGitlabToken("");
    await vi.waitFor(() => expect(order).toEqual(["update_settings"]));
    // The URL save is still on its way: the token waits for it.
    await Promise.resolve();
    expect(order).toEqual(["update_settings"]);
    release();
    expect(await Promise.all([saved, removed])).toEqual([true, true]);
    expect(order).toEqual(["update_settings", "set_gitlab_token"]);
  });

  it("re-checks the providers after every refresh", async () => {
    const loggedOut = status({
      github: {
        enabled: true,
        host: "github.com",
        installed: true,
        loggedIn: false,
      },
    });
    backend(loggedOut, false);
    const off = await subscribeBackendSettingsEvents();
    events.emit("reviews:refreshed", {
      changed: [],
      errors: [],
      refreshedAt: "2026-10-16T10:00:00Z",
    });
    await vi.waitFor(() =>
      expect(useBackendSettingsStore.getState().status).toEqual(loggedOut),
    );
    off();
  });
  it("runs overlapping saves one after another, each over the last", async () => {
    backend(status(), false);
    await useBackendSettingsStore.getState().load();
    const answer = mockInvoke.getMockImplementation()!;
    const sent: Settings[] = [];
    const pending: (() => void)[] = [];
    mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd !== "update_settings") return answer(cmd, args as never);
      const body = (args as { settings: Settings }).settings;
      sent.push(body);
      return new Promise((r) => pending.push(() => r(body)));
    });
    const store = useBackendSettingsStore.getState();
    const first = store.save({ pollIntervalSecs: 240 });
    // The stepper reads the store: the first click shows at once.
    expect(useBackendSettingsStore.getState().settings?.pollIntervalSecs).toBe(
      240,
    );
    const second = store.save({
      pollIntervalSecs:
        useBackendSettingsStore.getState().settings!.pollIntervalSecs + 60,
    });
    const third = store.save({
      gitlab: { baseUrl: "https://gl.example.test" },
    });
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    pending[0]();
    await vi.waitFor(() => expect(sent).toHaveLength(2));
    expect(sent[1].pollIntervalSecs).toBe(300);
    pending[1]();
    await vi.waitFor(() => expect(sent).toHaveLength(3));
    pending[2]();
    expect(await Promise.all([first, second, third])).toEqual([
      true,
      true,
      true,
    ]);
    expect(sent[2]).toMatchObject({
      pollIntervalSecs: 300,
      gitlab: { baseUrl: "https://gl.example.test" },
    });
    expect(useBackendSettingsStore.getState().settings).toMatchObject({
      pollIntervalSecs: 300,
      gitlab: { baseUrl: "https://gl.example.test" },
    });
  });
});
