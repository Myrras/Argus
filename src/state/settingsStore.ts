/**
 * Settings and provider status. The GitLab token is write-only from here:
 * it goes to `set_gitlab_token` and nowhere else (never into `Settings`),
 * and only whether one is stored ever comes back.
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
import type { IpcError, ProviderStatus, Settings } from "../ipc/types";

/** What to do with the stored GitLab token on save. */
export type TokenChange =
  { action: "keep" } | { action: "set"; token: string } | { action: "clear" };

interface SettingsStoreState {
  settings: Settings | null;
  tokenPresent: boolean;
  status: ProviderStatus | null;
  saving: boolean;
  lastError: IpcError | null;
  load: () => Promise<void>;
  /** Re-read the provider status and whether a token is stored. */
  refreshStatus: () => Promise<void>;
  /** Save settings, then apply the token change; resolves true on success. */
  save: (settings: Settings, token: TokenChange) => Promise<boolean>;
}

export const useSettingsStore = create<SettingsStoreState>((set, get) => ({
  settings: null,
  tokenPresent: false,
  status: null,
  saving: false,
  lastError: null,

  load: async () => {
    try {
      set({ settings: await getSettings(), lastError: null });
    } catch (e) {
      set({ lastError: toIpcError(e) });
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
      set({ lastError: toIpcError(e) });
    }
  },

  save: async (settings, token) => {
    set({ saving: true, lastError: null });
    try {
      // Settings first: the token is stored for the GitLab URL just saved.
      const stored = await updateSettings(settings);
      set({ settings: stored });
      if (token.action === "set") await setGitlabToken(token.token);
      if (token.action === "clear") await setGitlabToken("");
      await get().refreshStatus();
      return true;
    } catch (e) {
      set({ lastError: toIpcError(e) });
      return false;
    } finally {
      set({ saving: false });
    }
  },
}));

/**
 * After every refresh, re-check the providers (a `gh auth login` or a new
 * token shows up without reopening Settings). Resolves to an unsubscribe.
 */
export function subscribeSettingsEvents(): Promise<() => void> {
  return onReviewsRefreshed(
    () => void useSettingsStore.getState().refreshStatus(),
  );
}
