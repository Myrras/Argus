/**
 * The core's persisted settings (`settings.json`: GitHub, GitLab, the
 * refresh interval, AI, local clones), the provider status and whether a
 * GitLab token is stored — mirrored for the Settings dialog and the
 * sidebar's Providers section (Iris's `backendSettingsStore`).
 *
 * `load` re-reads them (on start); `save` writes a patch over what is stored
 * and keeps what the core stored (it normalizes). Saves run one at a time,
 * each over the last one's result, and show at once (a stepper clicked
 * three times saves all three steps). The GitLab token is
 * write-only from here: `setGitlabToken` hands it to `set_gitlab_token` and
 * nowhere else (never into `Settings`), and only whether one is stored ever
 * comes back. Failures are error toasts. UI-only preferences live in
 * `settingsStore` instead.
 *
 * `setupChanged` notes when a provider's settings or token last changed
 * (when the change was asked for), so the sidebar can drop errors of a
 * refresh that began before it (`lib/banner`'s `currentErrors`).
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
import type { ProviderId, ProviderStatus, Settings } from "../ipc/types";
import type { SetupChanges } from "../lib/banner";
import { toastError } from "./toastStore";

/** Bounds of the refresh interval in minutes (the core floors it at 30 s). */
export const POLL_MINUTES_MIN = 1;
export const POLL_MINUTES_MAX = 120;

interface BackendSettingsState {
  /** `null` until the first load arrived. */
  settings: Settings | null;
  tokenPresent: boolean;
  status: ProviderStatus | null;
  /** When each provider's settings or token last changed (ms). */
  setupChanged: SetupChanges;
  load: () => Promise<void>;
  /** Re-read the provider status and whether a token is stored. */
  refreshStatus: () => Promise<void>;
  /** Save a patch over the stored settings; resolves true on success. */
  save: (patch: Partial<Settings>) => Promise<boolean>;
  /** Store the GitLab token in the keychain (blank deletes it). */
  setGitlabToken: (token: string) => Promise<boolean>;
}

/** The running saves, in order, and their patches not yet stored. */
let saving: Promise<unknown> = Promise.resolve();
const queued: Partial<Settings>[] = [];

/** The providers whose setup a settings patch changes. */
const PROVIDER_KEYS: readonly ProviderId[] = ["github", "gitlab"];

function changedBy(
  patch: Partial<Settings>,
  at: number,
  before: SetupChanges,
): SetupChanges {
  const touched = PROVIDER_KEYS.filter((p) => p in patch);
  if (touched.length === 0) return before;
  return { ...before, ...Object.fromEntries(touched.map((p) => [p, at])) };
}

/** `base` with the patches still waiting on top (what the user sees). */
function withQueued(base: Settings): Settings {
  return Object.assign({ ...base }, ...queued) as Settings;
}

export const useBackendSettingsStore = create<BackendSettingsState>(
  (set, get) => ({
    settings: null,
    tokenPresent: false,
    status: null,
    setupChanged: {},

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

    save: (patch) => {
      if (!get().settings) return Promise.resolve(false);
      queued.push(patch);
      set((s) => ({ settings: s.settings && { ...s.settings, ...patch } }));
      const run = async (): Promise<boolean> => {
        // Built from the previous save's result, not from what the store
        // held when this one was asked for.
        const current = get().settings;
        try {
          if (!current) throw new Error("The settings are not loaded.");
          // Before the call: the refresh the save starts begins after it.
          const asked = Date.now();
          const stored = await updateSettings({ ...current, ...patch });
          queued.shift();
          set((s) => ({
            settings: withQueued(stored),
            setupChanged: changedBy(patch, asked, s.setupChanged),
          }));
        } catch (e) {
          queued.shift();
          toastError("Could not save the settings", toIpcError(e).message);
          await get().load();
          const loaded = get().settings;
          if (loaded) set({ settings: withQueued(loaded) });
          return false;
        }
        await get().refreshStatus();
        return true;
      };
      const p = saving.then(run);
      saving = p.catch(() => undefined);
      return p;
    },

    setGitlabToken: async (token) => {
      const asked = Date.now();
      try {
        await setGitlabToken(token);
        set((s) => ({ setupChanged: { ...s.setupChanged, gitlab: asked } }));
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
