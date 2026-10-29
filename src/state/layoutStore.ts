/**
 * Persistent shell layout (Iris's `layoutStore`): sidebar width, the
 * Providers section's collapse, the review view's file tree (open, tree
 * or flat) and diff layout (unified or side by side), whether whitespace
 * changes are hidden in every diff (`lib/whitespaceDiff`), and whether the
 * AI island (findings and checklist) is open.
 *
 * Builds with a Threads / Files tab strip saved `detailTab`; `load` ignores
 * it and the next save drops it.
 */

import { create } from "zustand";

/** The code view's diff: one column, or base and head side by side. */
export type DiffLayout = "unified" | "split";

/** The file tree as folders, or a flat list of paths. */
export type FileListMode = "tree" | "flat";

interface LayoutPersisted {
  sidebarWidth: number;
  /** Whether the sidebar's Providers section is collapsed to its title. */
  providersCollapsed: boolean;
  /** Whether the review view's file tree is expanded. */
  fileTreeOpen: boolean;
  diffLayout: DiffLayout;
  /** Diffs compare lines with whitespace ignored (GitHub's `?w=1`). */
  hideWhitespace: boolean;
  fileListMode: FileListMode;
  /** Whether the AI island right of the review is open. */
  aiPanelOpen: boolean;
}

export const SIDEBAR_MIN_WIDTH = 240;
export const SIDEBAR_MAX_WIDTH = 600;

const STORAGE_KEY = "argus.layout";

const DEFAULTS: LayoutPersisted = {
  sidebarWidth: 340,
  providersCollapsed: false,
  fileTreeOpen: true,
  diffLayout: "unified",
  hideWhitespace: false,
  fileListMode: "tree",
  aiPanelOpen: true,
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
      fileTreeOpen: saved.fileTreeOpen !== false,
      diffLayout: saved.diffLayout === "split" ? "split" : "unified",
      hideWhitespace: saved.hideWhitespace === true,
      fileListMode: saved.fileListMode === "flat" ? "flat" : "tree",
      aiPanelOpen: saved.aiPanelOpen !== false,
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
        fileTreeOpen: state.fileTreeOpen,
        diffLayout: state.diffLayout,
        hideWhitespace: state.hideWhitespace,
        fileListMode: state.fileListMode,
        aiPanelOpen: state.aiPanelOpen,
      }),
    );
  } catch {
    /* quota exceeded */
  }
}

interface LayoutState extends LayoutPersisted {
  setSidebarWidth: (width: number) => void;
  setProvidersCollapsed: (collapsed: boolean) => void;
  toggleFileTree: () => void;
  setDiffLayout: (layout: DiffLayout) => void;
  setHideWhitespace: (hide: boolean) => void;
  toggleHideWhitespace: () => void;
  setFileListMode: (mode: FileListMode) => void;
  setAiPanelOpen: (open: boolean) => void;
  toggleAiPanel: () => void;
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
    toggleFileTree: () => update({ fileTreeOpen: !get().fileTreeOpen }),
    setDiffLayout: (layout) => update({ diffLayout: layout }),
    setHideWhitespace: (hide) => update({ hideWhitespace: hide }),
    toggleHideWhitespace: () =>
      update({ hideWhitespace: !get().hideWhitespace }),
    setFileListMode: (mode) => update({ fileListMode: mode }),
    setAiPanelOpen: (open) => update({ aiPanelOpen: open }),
    toggleAiPanel: () => update({ aiPanelOpen: !get().aiPanelOpen }),
  };
});
