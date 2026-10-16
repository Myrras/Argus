import { beforeEach, describe, expect, it, vi } from "vitest";

async function freshStore() {
  vi.resetModules();
  return (await import("./layoutStore")).useLayoutStore;
}

describe("layoutStore", () => {
  beforeEach(() => localStorage.clear());

  it("clamps and persists the sidebar width", async () => {
    const store = await freshStore();
    store.getState().setSidebarWidth(5000);
    expect(store.getState().sidebarWidth).toBe(600);
    store.getState().setSidebarWidth(10);
    expect(store.getState().sidebarWidth).toBe(240);
    const reloaded = await freshStore();
    expect(reloaded.getState().sidebarWidth).toBe(240);
  });

  it("remembers the Providers section's collapse across reloads", async () => {
    const store = await freshStore();
    expect(store.getState().providersCollapsed).toBe(false);
    store.getState().setProvidersCollapsed(true);
    const reloaded = await freshStore();
    expect(reloaded.getState().providersCollapsed).toBe(true);
  });

  it("remembers the detail tab and the file tree", async () => {
    const store = await freshStore();
    expect(store.getState().detailTab).toBe("threads");
    expect(store.getState().fileTreeOpen).toBe(true);
    store.getState().setDetailTab("files");
    store.getState().toggleFileTree();
    const reloaded = await freshStore();
    expect(reloaded.getState().detailTab).toBe("files");
    expect(reloaded.getState().fileTreeOpen).toBe(false);
  });

  it("remembers the diff layout and the file list mode", async () => {
    const store = await freshStore();
    expect(store.getState().diffLayout).toBe("unified");
    expect(store.getState().fileListMode).toBe("tree");
    store.getState().setDiffLayout("split");
    store.getState().setFileListMode("flat");
    const reloaded = await freshStore();
    expect(reloaded.getState().diffLayout).toBe("split");
    expect(reloaded.getState().fileListMode).toBe("flat");
  });

  it("falls back to the defaults for unknown or missing values", async () => {
    localStorage.setItem(
      "argus.layout",
      JSON.stringify({
        detailTab: "diff",
        sidebarWidth: "wide",
        diffLayout: "3d",
      }),
    );
    const store = await freshStore();
    expect(store.getState().detailTab).toBe("threads");
    expect(store.getState().sidebarWidth).toBe(340);
    expect(store.getState().fileTreeOpen).toBe(true);
    expect(store.getState().diffLayout).toBe("unified");
  });
});
