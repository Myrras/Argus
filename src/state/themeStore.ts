/** Theme selection, persisted to localStorage and applied to <html>. */

import { create } from "zustand";

import { prefersReducedMotion } from "../lib/motion";

export type ThemeMode = "dark" | "light";

/** `document` augmented with the (still newish) View Transitions API. */
type ViewTransitionDocument = Document & {
  startViewTransition?: (callback: () => void) => unknown;
};

const STORAGE_KEY = "argus.theme";

function initialTheme(): ThemeMode {
  return localStorage.getItem(STORAGE_KEY) === "light" ? "light" : "dark";
}

function applyTheme(mode: ThemeMode): void {
  document.documentElement.setAttribute("data-theme", mode);
}

interface ThemeState {
  mode: ThemeMode;
  toggle: () => void;
}

export const useThemeStore = create<ThemeState>((set, get) => {
  // Apply once at store creation so the UI never flashes the wrong theme.
  applyTheme(initialTheme());
  return {
    mode: initialTheme(),
    toggle: () => {
      const next: ThemeMode = get().mode === "dark" ? "light" : "dark";
      localStorage.setItem(STORAGE_KEY, next);
      // The palette swap is the synchronous `data-theme` flip; flipping it
      // inside a view transition lets the WebView crossfade the whole UI.
      // Falls back to an instant swap when the API is missing or the user
      // prefers reduced motion.
      const apply = () => {
        applyTheme(next);
        set({ mode: next });
      };
      const doc = document as ViewTransitionDocument;
      if (
        typeof doc.startViewTransition === "function" &&
        !prefersReducedMotion()
      ) {
        doc.startViewTransition(apply);
      } else {
        apply();
      }
    },
  };
});
