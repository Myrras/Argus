import { beforeEach, describe, expect, it, vi } from "vitest";

import { fakeEvents, settings, status } from "../test/fixtures";

const events = fakeEvents();
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) =>
    events.listen(name, cb),
  ),
}));

import { invoke } from "@tauri-apps/api/core";

import type { ProviderStatus } from "../ipc/types";
import { subscribeSettingsEvents, useSettingsStore } from "./settingsStore";

const mockInvoke = vi.mocked(invoke);
const initial = useSettingsStore.getState();

function backend(st: ProviderStatus, tokenPresent: boolean) {
  mockInvoke.mockImplementation((cmd: string) => {
    switch (cmd) {
      case "get_settings":
        return Promise.resolve(settings());
      case "provider_status":
        return Promise.resolve(st);
      case "gitlab_token_present":
        return Promise.resolve(tokenPresent);
      default:
        return Promise.resolve(undefined);
    }
  });
}

describe("settingsStore", () => {
  beforeEach(() => {
    useSettingsStore.setState({ ...initial }, true);
  });

  it("loads settings, provider status and token presence", async () => {
    backend(status(), true);
    await useSettingsStore.getState().load();
    expect(useSettingsStore.getState()).toMatchObject({
      settings: settings(),
      status: status(),
      tokenPresent: true,
      lastError: null,
    });
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
    const off = await subscribeSettingsEvents();
    events.emit("reviews:refreshed", {
      changed: [],
      errors: [],
      refreshedAt: "2026-10-16T10:00:00Z",
    });
    await vi.waitFor(() =>
      expect(useSettingsStore.getState().status).toEqual(loggedOut),
    );
    off();
  });

  it("never puts the token in update_settings", async () => {
    backend(status(), true);
    const ok = await useSettingsStore
      .getState()
      .save(settings(), { action: "set", token: "glpat-fixture" });
    expect(ok).toBe(true);
    const update = mockInvoke.mock.calls.find(([c]) => c === "update_settings");
    expect(update?.[1]).toEqual({ settings: settings() });
    expect(JSON.stringify(update)).not.toContain("glpat-fixture");
    expect(mockInvoke).toHaveBeenCalledWith("set_gitlab_token", {
      token: "glpat-fixture",
    });
    expect(useSettingsStore.getState().tokenPresent).toBe(true);
  });
});
