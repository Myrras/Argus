/**
 * One-shot "open Settings on this tab" request, so a component deep in the
 * tree (the Providers section's "Settings" button, the clone chip) can reach
 * the modal `App` owns. Settings → Repositories also takes the repository
 * (`host/repo`), Settings → Accounts the account (`provider:host`), to
 * scroll to and highlight.
 */

import { create } from "zustand";

/**
 * Settings tabs: General (theme, reduced motion, refresh interval),
 * Accounts (the hosts gh and glab are logged in to), AI (the analysis and
 * its default language),
 * Repositories (per repository: local clone, review skill, review language),
 * Editors (which app opens a clone's file), Scripts (the submit scripts) and
 * Shortcuts (rebindable keys).
 */
export type SettingsTabId =
  | "general"
  | "accounts"
  | "ai"
  | "repositories"
  | "editors"
  | "scripts"
  | "shortcuts";

interface SettingsRequestState {
  request: SettingsTabId | null;
  /** The repository (`host/repo`) Settings → Repositories should show. */
  repo: string | null;
  /** The account (`provider:host`) Settings → Accounts should show. */
  account: string | null;
  /** Counts the requests: the same one asked again still opens its tab. */
  seq: number;
  /** `target`: the repository or account to show, for those tabs. */
  open: (tab: SettingsTabId, target?: string | null) => void;
  clear: () => void;
}

export const useSettingsRequestStore = create<SettingsRequestState>((set) => ({
  request: null,
  repo: null,
  account: null,
  seq: 0,
  open: (tab, target = null) =>
    set((s) => ({
      request: tab,
      repo: tab === "repositories" ? target : null,
      account: tab === "accounts" ? target : null,
      seq: s.seq + 1,
    })),
  clear: () => set({ request: null, repo: null, account: null }),
}));
