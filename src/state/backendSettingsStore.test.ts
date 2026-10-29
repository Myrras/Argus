import { beforeEach, describe, expect, it, vi } from "vitest";

import { account, accounts, fakeEvents, settings } from "../test/fixtures";

const events = fakeEvents();
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) =>
    events.listen(name, cb),
  ),
}));
vi.mock("./toastStore", () => ({ toastError: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";

import type { Account, Settings } from "../ipc/types";
import {
  subscribeBackendSettingsEvents,
  useBackendSettingsStore,
} from "./backendSettingsStore";
import { toastError } from "./toastStore";

const mockInvoke = vi.mocked(invoke);
const initial = useBackendSettingsStore.getState();
function backend(st: Account[]) {
  mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
    switch (cmd) {
      case "get_settings":
        return Promise.resolve(settings());
      case "update_settings":
        return Promise.resolve((args as { settings: Settings }).settings);
      case "list_accounts":
        return Promise.resolve(st);
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

  it("loads settings and the accounts", async () => {
    backend(accounts());
    await useBackendSettingsStore.getState().load();
    expect(useBackendSettingsStore.getState()).toMatchObject({
      settings: settings(),
      accounts: accounts(),
    });
    // No token anywhere: gh and glab keep theirs.
    expect(mockInvoke.mock.calls.map(([c]) => c).sort()).toEqual([
      "get_settings",
      "list_accounts",
    ]);
    expect(mockInvoke).toHaveBeenCalledWith("list_accounts", {
      recheck: false,
    });
  });

  it("saves a patch over the stored settings, then re-reads the accounts", async () => {
    backend(accounts());
    await useBackendSettingsStore.getState().load();
    mockInvoke.mockClear();
    const ok = await useBackendSettingsStore
      .getState()
      .save({ pollIntervalSecs: 300 });
    expect(ok).toBe(true);
    expect(mockInvoke).toHaveBeenCalledWith("update_settings", {
      settings: { ...settings(), pollIntervalSecs: 300 },
    });
    expect(mockInvoke).toHaveBeenCalledWith("list_accounts", {
      recheck: false,
    });
    expect(useBackendSettingsStore.getState().settings?.pollIntervalSecs).toBe(
      300,
    );
  });

  it("re-reads the accounts as the core keeps them; Check again asks the CLIs", async () => {
    backend(accounts());
    await useBackendSettingsStore.getState().refreshAccounts();
    expect(mockInvoke.mock.calls).toEqual([
      ["list_accounts", { recheck: false }],
    ]);
    mockInvoke.mockClear();
    const fresh = [account({ state: { kind: "notLoggedIn", message: null } })];
    mockInvoke.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === "list_accounts" ? fresh : undefined),
    );
    await useBackendSettingsStore.getState().checkAccounts();
    expect(mockInvoke.mock.calls).toEqual([
      ["list_accounts", { recheck: true }],
    ]);
    expect(useBackendSettingsStore.getState().accounts).toEqual(fresh);
  });

  it("switches a host at once and saves its choice", async () => {
    backend(accounts());
    await useBackendSettingsStore.getState().load();
    mockInvoke.mockClear();
    const saved = useBackendSettingsStore
      .getState()
      .setAccount("gitlab:gitlab.example.test", {
        enabled: false,
        account: null,
      });
    // The switch moves before the core answers.
    expect(
      useBackendSettingsStore
        .getState()
        .accounts?.find((a) => a.key === "gitlab:gitlab.example.test")?.enabled,
    ).toBe(false);
    expect(await saved).toBe(true);
    expect(mockInvoke).toHaveBeenCalledWith("update_settings", {
      settings: {
        ...settings(),
        accounts: {
          "gitlab:gitlab.example.test": { enabled: false, account: null },
        },
      },
    });
  });

  it("reports a rejected save and re-reads the stored value", async () => {
    backend(accounts());
    await useBackendSettingsStore.getState().load();
    mockInvoke.mockImplementationOnce(() =>
      Promise.reject({ kind: "io", message: "the disk is full" }),
    );
    const ok = await useBackendSettingsStore.getState().save({
      accounts: { "github:github.com": { enabled: false, account: null } },
    });
    expect(ok).toBe(false);
    expect(toastError).toHaveBeenCalledWith(
      "Could not save the settings",
      "the disk is full",
    );
    expect(useBackendSettingsStore.getState().settings).toEqual(settings());
  });

  it("notes when a provider's accounts changed, as asked for", async () => {
    backend(accounts());
    await useBackendSettingsStore.getState().load();
    await useBackendSettingsStore.getState().save({ pollIntervalSecs: 300 });
    expect(useBackendSettingsStore.getState().setupChanged).toEqual({});

    const before = Date.now();
    await useBackendSettingsStore.getState().save({
      accounts: { "gitlab:gitlab.vpn.test": { enabled: false, account: null } },
    });
    const saved = useBackendSettingsStore.getState().setupChanged;
    expect(Object.keys(saved)).toEqual(["gitlab"]);
    expect(saved.gitlab).toBeGreaterThanOrEqual(before);

    // A save the core refused changes nothing.
    mockInvoke.mockRejectedValueOnce({ kind: "invalidInput", message: "nope" });
    await useBackendSettingsStore.getState().save({
      accounts: { "github:github.com": { enabled: false, account: null } },
    });
    expect(useBackendSettingsStore.getState().setupChanged).toEqual(saved);
  });

  it("re-reads the accounts after every refresh", async () => {
    const loggedOut = [
      account({ state: { kind: "notLoggedIn", message: null } }),
    ];
    backend(loggedOut);
    const off = await subscribeBackendSettingsEvents();
    events.emit("reviews:refreshed", {
      changed: [],
      errors: [],
      refreshedAt: "2026-10-16T10:00:00Z",
    });
    await vi.waitFor(() =>
      expect(useBackendSettingsStore.getState().accounts).toEqual(loggedOut),
    );
    off();
  });

  it("runs overlapping saves one after another, each over the last", async () => {
    backend(accounts());
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
      accounts: { "gitlab:gl.example.test": { enabled: false, account: null } },
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
    const off = {
      "gitlab:gl.example.test": { enabled: false, account: null },
    };
    expect(sent[2]).toMatchObject({ pollIntervalSecs: 300, accounts: off });
    expect(useBackendSettingsStore.getState().settings).toMatchObject({
      pollIntervalSecs: 300,
      accounts: off,
    });
  });
});
