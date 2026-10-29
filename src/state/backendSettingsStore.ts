/**
 * The core's persisted settings (`settings.json`: the account choices, the
 * refresh interval, AI, local clones) and the user's accounts (what gh and
 * glab are logged in to) — mirrored for the Settings dialog and the
 * sidebar's Providers section (Iris's `backendSettingsStore`). No token
 * passes here: GitHub's stays with `gh`, GitLab's with `glab`.
 *
 * `load` re-reads them (on start); `save` writes a patch over what is stored
 * and keeps what the core stored (it normalizes). Saves run one at a time,
 * each over the last one's result, and show at once (a stepper clicked
 * three times saves all three steps). Failures are error toasts. UI-only
 * preferences live in `settingsStore` instead.
 *
 * `setupChanged` notes when a provider's accounts last changed (when the
 * change was asked for), so the sidebar can drop errors of a refresh that
 * began before it (`lib/banner`'s `currentErrors`).
 */

import { create } from "zustand";

import {
  getSettings,
  listAccounts,
  toIpcError,
  updateSettings,
} from "../ipc/commands";
import { onReviewsRefreshed } from "../ipc/events";
import type {
  Account,
  AccountChoice,
  ProviderId,
  Settings,
} from "../ipc/types";
import { changedProviders, withChoice } from "../lib/accounts";
import type { SetupChanges } from "../lib/banner";
import { toastError } from "./toastStore";

/** Bounds of the refresh interval in minutes (the core floors it at 30 s). */
export const POLL_MINUTES_MIN = 1;
export const POLL_MINUTES_MAX = 120;

interface BackendSettingsState {
  /** `null` until the first load arrived. */
  settings: Settings | null;
  /** `null` until the first read arrived. */
  accounts: Account[] | null;
  /** When each provider's accounts last changed (ms). */
  setupChanged: SetupChanges;
  load: () => Promise<void>;
  /**
   * Re-read the accounts (the core asks gh and glab only when its answer
   * is not current).
   */
  refreshAccounts: () => Promise<void>;
  /** Re-read them, asking gh and glab in any case (Check again). */
  checkAccounts: () => Promise<void>;
  /** Save a patch over the stored settings; resolves true on success. */
  save: (patch: Partial<Settings>) => Promise<boolean>;
  /** Save one host's choice (its switch, the gh account picker). */
  setAccount: (key: string, choice: AccountChoice) => Promise<boolean>;
}

/** The running saves, in order, and their patches not yet stored. */
let saving: Promise<unknown> = Promise.resolve();
const queued: Partial<Settings>[] = [];

/** `before` with `touched` (providers whose accounts changed) at `at`. */
function changedBy(
  touched: readonly ProviderId[],
  at: number,
  before: SetupChanges,
): SetupChanges {
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
    accounts: null,
    setupChanged: {},

    load: async () => {
      try {
        set({ settings: await getSettings() });
      } catch (e) {
        toastError("Could not load the settings", toIpcError(e).message);
      }
      await get().refreshAccounts();
    },

    refreshAccounts: async () => {
      try {
        set({ accounts: await listAccounts(false) });
      } catch (e) {
        toastError("Could not list the accounts", toIpcError(e).message);
      }
    },

    checkAccounts: async () => {
      try {
        set({ accounts: await listAccounts(true) });
      } catch (e) {
        toastError("Could not list the accounts", toIpcError(e).message);
      }
    },

    setAccount: (key, choice) => {
      const current = get().settings;
      if (!current) return Promise.resolve(false);
      // Shown at once, before the save: the switch moves on click.
      set((s) => ({
        accounts:
          s.accounts?.map((a) =>
            a.key === key
              ? { ...a, enabled: choice.enabled, account: choice.account }
              : a,
          ) ?? null,
      }));
      return get().save({
        accounts: withChoice(current.accounts, key, choice),
      });
    },

    save: (patch) => {
      const shown = get().settings;
      if (!shown) return Promise.resolve(false);
      // Against what the user saw before this change (the store shows it
      // at once, below).
      const touched = patch.accounts
        ? changedProviders(shown.accounts, patch.accounts)
        : [];
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
            setupChanged: changedBy(touched, asked, s.setupChanged),
          }));
        } catch (e) {
          queued.shift();
          toastError("Could not save the settings", toIpcError(e).message);
          await get().load();
          const loaded = get().settings;
          if (loaded) set({ settings: withQueued(loaded) });
          return false;
        }
        await get().refreshAccounts();
        return true;
      };
      const p = saving.then(run);
      saving = p.catch(() => undefined);
      return p;
    },
  }),
);

/**
 * After every refresh, re-read the accounts (as the refresh's calls left
 * them; gh and glab asked again only after a call was refused for the
 * login). Resolves to an unsubscribe.
 */
export function subscribeBackendSettingsEvents(): Promise<() => void> {
  return onReviewsRefreshed(
    () => void useBackendSettingsStore.getState().refreshAccounts(),
  );
}
