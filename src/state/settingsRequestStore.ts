/**
 * One-shot "open Settings on this tab" request, so a component deep in the
 * tree (the Providers section's "Settings" button) can reach the modal `App`
 * owns.
 */

import { create } from "zustand";

/**
 * Settings tabs: General (theme, reduced motion, refresh interval), GitHub
 * and GitLab (the providers), AI (analysis and local clones) and Shortcuts
 * (rebindable keys).
 */
export type SettingsTabId =
  "general" | "github" | "gitlab" | "ai" | "shortcuts";

interface SettingsRequestState {
  request: SettingsTabId | null;
  open: (tab: SettingsTabId) => void;
  clear: () => void;
}

export const useSettingsRequestStore = create<SettingsRequestState>((set) => ({
  request: null,
  open: (tab) => set({ request: tab }),
  clear: () => set({ request: null }),
}));
