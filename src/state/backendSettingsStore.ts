/**
 * The core's persisted settings (`settings.json`: GitHub, GitLab, the
 * refresh interval, AI, local clones), the provider status and whether a
 * GitLab token is stored — mirrored for the Settings dialog and the
 * sidebar's Providers section (Iris's `backendSettingsStore`).
 *
 * `load` re-reads them (on start); `save` writes a patch over what is stored
 * and keeps what the core stored (it normalizes). The GitLab token is
 * write-only from here: `setGitlabToken` hands it to `set_gitlab_token` and
 * nowhere else (never into `Settings`), and only whether one is stored ever
 * comes back. Failures are error toasts. UI-only preferences live in
 * `settingsStore` instead.
 */

import { create } from "zustand";

import {
  getSettings,
  gitlabTokenPresent,
  providerStatus,
  setGitlabToken,
  toIpcError,
  updateSettings,
} from "../ipc/commands";
import { onReviewsRefreshed } from "../ipc/events";
import type { ProviderStatus, Settings } from "../ipc/types";
import { toastError } from "./toastStore";

/** Bounds of the refresh interval in minutes (the core floors it at 30 s). */
export const POLL_MINUTES_MIN = 1;
export const POLL_MINUTES_MAX = 120;

interface BackendSettingsState {
  /** `null` until the first load arrived. */
  settings: Settings | null;
  tokenPresent: boolean;
  status: ProviderStatus | null;
  load: () => Promise<void>;
  /** Re-read the provider status and whether a token is stored. */
  refreshStatus: () => Promise<void>;
  /** Save a patch over the stored settings; resolves true on success. */
  save: (patch: Partial<Settings>) => Promise<boolean>;
  /** Store the GitLab token in the keychain (blank deletes it). */
  setGitlabToken: (token: string) => Promise<boolean>;
}

export const useBackendSettingsStore = create<BackendSettingsState>(
  (set, get) => ({
    settings: null,
    tokenPresent: false,
    status: null,

    load: async () => {
      try {
        set({ settings: await getSettings() });
      } catch (e) {
        toastError("Could not load the settings", toIpcError(e).message);
      }
      await get().refreshStatus();
    },

    refreshStatus: async () => {
      try {
        const [status, tokenPresent] = await Promise.all([
          providerStatus(),
          gitlabTokenPresent(),
        ]);
        set({ status, tokenPresent });
      } catch (e) {
        toastError("Could not check the providers", toIpcError(e).message);
      }
    },

    save: async (patch) => {
      const current = get().settings;
      if (!current) return false;
      try {
        set({ settings: await updateSettings({ ...current, ...patch }) });
      } catch (e) {
        toastError("Could not save the settings", toIpcError(e).message);
        await get().load();
        return false;
      }
      await get().refreshStatus();
      return true;
    },

    setGitlabToken: async (token) => {
      try {
        await setGitlabToken(token);
      } catch (e) {
        toastError(
          token ? "Could not store the token" : "Could not remove the token",
          toIpcError(e).message,
        );
        return false;
      }
      await get().refreshStatus();
      return true;
    },
  }),
);

/**
 * After every refresh, re-check the providers (a `gh auth login` or a new
 * token shows up without reopening Settings). Resolves to an unsubscribe.
 */
export function subscribeBackendSettingsEvents(): Promise<() => void> {
  return onReviewsRefreshed(
    () => void useBackendSettingsStore.getState().refreshStatus(),
  );
}
