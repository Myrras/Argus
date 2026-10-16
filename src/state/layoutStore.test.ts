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

  it("remembers the file tree", async () => {
    const store = await freshStore();
    expect(store.getState().fileTreeOpen).toBe(true);
    store.getState().toggleFileTree();
    const reloaded = await freshStore();
    expect(reloaded.getState().fileTreeOpen).toBe(false);
  });

  it("ignores the old detail tab and drops it on the next save", async () => {
    localStorage.setItem(
      "argus.layout",
      JSON.stringify({ detailTab: "files", diffLayout: "split" }),
    );
    const store = await freshStore();
    expect(store.getState().diffLayout).toBe("split");
    expect("detailTab" in store.getState()).toBe(false);
    store.getState().toggleFileTree();
    const saved = JSON.parse(localStorage.getItem("argus.layout") ?? "{}");
    expect(saved).not.toHaveProperty("detailTab");
    expect(saved.diffLayout).toBe("split");
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
        sidebarWidth: "wide",
        diffLayout: "3d",
      }),
    );
    const store = await freshStore();
    expect(store.getState().sidebarWidth).toBe(340);
    expect(store.getState().fileTreeOpen).toBe(true);
    expect(store.getState().diffLayout).toBe("unified");
  });
});
