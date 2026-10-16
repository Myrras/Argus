/**
 * UI preferences, persisted to localStorage. Deep-merged over DEFAULTS
 * (Iris's `settingsStore`).
 *
 * Only what the frontend alone needs lives here; backend-relevant settings
 * (providers, refresh interval, AI, clones) are the core's `settings.json`
 * via `backendSettingsStore`. The theme has its own store (`themeStore`), as
 * in Podium and Iris.
 */

import { create } from "zustand";

import type { ActionId, ShortcutOverrides } from "../lib/keymap";

export interface Settings {
  appearance: { reduceMotion: boolean };
  /**
   * Only the shortcuts the user changed (action → its bindings); the rest
   * follow `lib/keymap`'s defaults, also for actions added later.
   */
  shortcuts: ShortcutOverrides;
}

export const DEFAULTS: Settings = {
  appearance: { reduceMotion: false },
  shortcuts: {},
};

const STORAGE_KEY = "argus.settings";

/** Keeps the entries that are a non-empty list of strings for a known action. */
function loadShortcuts(saved: unknown): ShortcutOverrides {
  const out: ShortcutOverrides = {};
  if (!saved || typeof saved !== "object") return out;
  for (const [action, list] of Object.entries(saved)) {
    if (
      Array.isArray(list) &&
      list.length > 0 &&
      list.every((b) => typeof b === "string")
    ) {
      out[action as ActionId] = list;
    }
  }
  return out;
}

function persist(s: Settings): void {
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ appearance: s.appearance, shortcuts: s.shortcuts }),
    );
  } catch {
    /* quota exceeded: keep the in-memory value for this session */
  }
}

function load(): Settings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return structuredClone(DEFAULTS);
    // Spread saved values over defaults per section, so a blob whose shape
    // predates a newer field still gets that field's default.
    const saved = JSON.parse(raw) as Partial<Settings>;
    return {
      appearance: { ...DEFAULTS.appearance, ...saved.appearance },
      shortcuts: loadShortcuts(saved.shortcuts),
    };
  } catch {
    return structuredClone(DEFAULTS);
  }
}

/** Sets the manual reduced-motion override on <html>. Turning the toggle on
 *  sets `data-reduce-motion="true"` (matched by the global.css guard); turning
 *  it off removes the attribute, so the OS-level media query still applies. */
export function applyReduceMotion(reduce: boolean): void {
  const el = document.documentElement;
  if (reduce) {
    el.dataset.reduceMotion = "true";
  } else {
    delete el.dataset.reduceMotion;
  }
}

interface SettingsState extends Settings {
  set: <K extends keyof Settings>(
    section: K,
    patch: Partial<Settings[K]>,
  ) => void;
  resetSettings: () => void;
  /** Rebinds an action; `null` puts its default back. */
  setShortcut: (action: ActionId, bindings: string[] | null) => void;
  /** Puts every shortcut back to its default. */
  resetShortcuts: () => void;
}

export const useSettingsStore = create<SettingsState>((set, get) => {
  const initial = load();
  applyReduceMotion(initial.appearance.reduceMotion);
  return {
    ...initial,
    set: (section, patch) => {
      const next = { ...get()[section], ...patch } as Settings[typeof section];
      persist({ ...get(), [section]: next });
      if (section === "appearance") {
        applyReduceMotion((next as Settings["appearance"]).reduceMotion);
      }
      set({ [section]: next } as Partial<SettingsState>);
    },
    resetSettings: () => {
      try {
        localStorage.removeItem(STORAGE_KEY);
      } catch {
        /* blocked storage: the defaults apply for this session */
      }
      applyReduceMotion(DEFAULTS.appearance.reduceMotion);
      set({ ...structuredClone(DEFAULTS) });
    },
    setShortcut: (action, bindings) => {
      const next = { ...get().shortcuts };
      if (bindings && bindings.length > 0) next[action] = bindings;
      else delete next[action];
      persist({ ...get(), shortcuts: next });
      set({ shortcuts: next });
    },
    resetShortcuts: () => {
      persist({ ...get(), shortcuts: {} });
      set({ shortcuts: {} });
    },
  };
});
