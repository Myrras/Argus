/**
 * Persistent shell layout (Iris's `layoutStore`): sidebar width, the
 * Providers section's collapse, the detail island's tab, and the Files tab's
 * file tree (open, tree or flat) and diff layout (unified or side by side).
 */

import { create } from "zustand";

/** The detail island's tabs: the threads, and the changed files (diff). */
export type DetailTab = "threads" | "files";

/** The code view's diff: one column, or base and head side by side. */
export type DiffLayout = "unified" | "split";

/** The file tree as folders, or a flat list of paths. */
export type FileListMode = "tree" | "flat";

interface LayoutPersisted {
  sidebarWidth: number;
  /** Whether the sidebar's Providers section is collapsed to its title. */
  providersCollapsed: boolean;
  detailTab: DetailTab;
  /** Whether the Files tab's file tree is expanded. */
  fileTreeOpen: boolean;
  diffLayout: DiffLayout;
  fileListMode: FileListMode;
}

export const SIDEBAR_MIN_WIDTH = 240;
export const SIDEBAR_MAX_WIDTH = 600;

const STORAGE_KEY = "argus.layout";

const DEFAULTS: LayoutPersisted = {
  sidebarWidth: 340,
  providersCollapsed: false,
  detailTab: "threads",
  fileTreeOpen: true,
  diffLayout: "unified",
  fileListMode: "tree",
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
      diffLayout: saved.diffLayout === "split" ? "split" : "unified",
      fileListMode: saved.fileListMode === "flat" ? "flat" : "tree",
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
        diffLayout: state.diffLayout,
        fileListMode: state.fileListMode,
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
  setDiffLayout: (layout: DiffLayout) => void;
  setFileListMode: (mode: FileListMode) => void;
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
    setDiffLayout: (layout) => update({ diffLayout: layout }),
    setFileListMode: (mode) => update({ fileListMode: mode }),
  };
});
