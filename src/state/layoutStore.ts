/**
 * Persistent shell layout (Iris's `layoutStore`): sidebar width, the
 * Providers section's collapse, the detail island's tab and whether the
 * Files tab's file tree is open.
 */

import { create } from "zustand";

/** The detail island's tabs: the threads, and the changed files (diff). */
export type DetailTab = "threads" | "files";

interface LayoutPersisted {
  sidebarWidth: number;
  /** Whether the sidebar's Providers section is collapsed to its title. */
  providersCollapsed: boolean;
  detailTab: DetailTab;
  /** Whether the Files tab's file tree is expanded. */
  fileTreeOpen: boolean;
}

export const SIDEBAR_MIN_WIDTH = 240;
export const SIDEBAR_MAX_WIDTH = 600;

const STORAGE_KEY = "argus.layout";

const DEFAULTS: LayoutPersisted = {
  sidebarWidth: 340,
  providersCollapsed: false,
  detailTab: "threads",
  fileTreeOpen: true,
};

function clampWidth(width: number): number {
  return Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, width));
}

function load(): LayoutPersisted {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...DEFAULTS };
    const saved = JSON.parse(raw) as Partial<LayoutPersisted>;
    return {
      sidebarWidth:
        typeof saved.sidebarWidth === "number"
          ? clampWidth(saved.sidebarWidth)
          : DEFAULTS.sidebarWidth,
      providersCollapsed: saved.providersCollapsed === true,
      detailTab: saved.detailTab === "files" ? "files" : "threads",
      fileTreeOpen: saved.fileTreeOpen !== false,
    };
  } catch {
    return { ...DEFAULTS };
  }
}

function persist(state: LayoutPersisted) {
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({
        sidebarWidth: state.sidebarWidth,
        providersCollapsed: state.providersCollapsed,
        detailTab: state.detailTab,
        fileTreeOpen: state.fileTreeOpen,
      }),
    );
  } catch {
    /* quota exceeded */
  }
}

interface LayoutState extends LayoutPersisted {
  setSidebarWidth: (width: number) => void;
  setProvidersCollapsed: (collapsed: boolean) => void;
  setDetailTab: (tab: DetailTab) => void;
  toggleFileTree: () => void;
}

export const useLayoutStore = create<LayoutState>((set, get) => {
  const update = (patch: Partial<LayoutPersisted>) => {
    set(patch);
    persist(get());
  };
  return {
    ...load(),
    setSidebarWidth: (width) => update({ sidebarWidth: clampWidth(width) }),
    setProvidersCollapsed: (collapsed) =>
      update({ providersCollapsed: collapsed }),
    setDetailTab: (tab) => update({ detailTab: tab }),
    toggleFileTree: () => update({ fileTreeOpen: !get().fileTreeOpen }),
  };
});
