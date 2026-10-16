import { beforeEach, describe, expect, it, vi } from "vitest";

// The store reads localStorage and applies the theme at creation, so each
// test imports a fresh module after arranging storage.
async function freshStore() {
  vi.resetModules();
  return (await import("./themeStore")).useThemeStore;
}

describe("themeStore", () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute("data-theme");
  });

  it("defaults to dark and applies it to <html>", async () => {
    const store = await freshStore();
    expect(store.getState().mode).toBe("dark");
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
  });

  it("restores a stored theme from argus.theme", async () => {
    localStorage.setItem("argus.theme", "retro");
    const store = await freshStore();
    expect(store.getState().mode).toBe("retro");
    expect(document.documentElement.getAttribute("data-theme")).toBe("retro");
  });

  it("ignores an unknown stored value", async () => {
    localStorage.setItem("argus.theme", "neon");
    const store = await freshStore();
    expect(store.getState().mode).toBe("dark");
  });

  it("persists and applies a new theme", async () => {
    const store = await freshStore();
    store.getState().setTheme("light");
    expect(store.getState().mode).toBe("light");
    expect(localStorage.getItem("argus.theme")).toBe("light");
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
  });

  it("swaps the palette inside a view transition when available", async () => {
    const store = await freshStore();
    const startViewTransition = vi.fn((cb: () => void) => cb());
    Object.assign(document, { startViewTransition });
    try {
      store.getState().setTheme("retro");
      expect(startViewTransition).toHaveBeenCalledTimes(1);
      expect(store.getState().mode).toBe("retro");
    } finally {
      delete (document as { startViewTransition?: unknown })
        .startViewTransition;
    }
  });
});
