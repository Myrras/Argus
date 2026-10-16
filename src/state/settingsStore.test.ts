import { beforeEach, describe, expect, it, vi } from "vitest";

async function freshStore() {
  vi.resetModules();
  return (await import("./settingsStore")).useSettingsStore;
}

describe("settingsStore", () => {
  beforeEach(() => {
    localStorage.clear();
    delete document.documentElement.dataset.reduceMotion;
  });

  it("starts from the defaults", async () => {
    const s = (await freshStore()).getState();
    expect(s.appearance.reduceMotion).toBe(false);
    expect(s.shortcuts).toEqual({});
  });

  it("deep-merges a saved blob over the defaults per section", async () => {
    // An older blob with an empty appearance section and no shortcuts.
    localStorage.setItem("argus.settings", JSON.stringify({ appearance: {} }));
    const s = (await freshStore()).getState();
    expect(s.appearance.reduceMotion).toBe(false);
    expect(s.shortcuts).toEqual({});
  });

  it("falls back to defaults on a corrupt blob", async () => {
    localStorage.setItem("argus.settings", "{not json");
    const s = (await freshStore()).getState();
    expect(s.appearance.reduceMotion).toBe(false);
  });

  it("persists a section patch under argus.settings", async () => {
    const store = await freshStore();
    store.getState().set("appearance", { reduceMotion: true });
    expect(store.getState().appearance.reduceMotion).toBe(true);
    const saved = JSON.parse(localStorage.getItem("argus.settings") ?? "{}");
    expect(saved.appearance.reduceMotion).toBe(true);
    expect((await freshStore()).getState().appearance.reduceMotion).toBe(true);
  });

  it("applies the reduce-motion override to <html>", async () => {
    const store = await freshStore();
    store.getState().set("appearance", { reduceMotion: true });
    expect(document.documentElement.dataset.reduceMotion).toBe("true");
    store.getState().set("appearance", { reduceMotion: false });
    expect(document.documentElement.dataset.reduceMotion).toBeUndefined();
  });

  it("resets to the defaults", async () => {
    const store = await freshStore();
    store.getState().set("appearance", { reduceMotion: true });
    store.getState().resetSettings();
    expect(store.getState().appearance.reduceMotion).toBe(false);
    expect(localStorage.getItem("argus.settings")).toBeNull();
  });

  it("keeps only the shortcuts the user changed, over the defaults", async () => {
    localStorage.setItem(
      "argus.settings",
      JSON.stringify({
        shortcuts: { refresh: ["Mod+KeyK"], openReview: [], nextReview: "x" },
      }),
    );
    const store = await freshStore();
    expect(store.getState().shortcuts).toEqual({ refresh: ["Mod+KeyK"] });
    const { resolveKeymap } = await import("../lib/keymap");
    const map = resolveKeymap(store.getState().shortcuts);
    expect(map.refresh).toEqual(["Mod+KeyK"]);
    expect(map.openReview).toEqual(["Enter"]);
  });

  it("sets, clears and resets shortcuts, persisting each", async () => {
    const store = await freshStore();
    const stored = () =>
      JSON.parse(localStorage.getItem("argus.settings")!).shortcuts;
    store.getState().setShortcut("refresh", ["Mod+KeyK"]);
    store.getState().setShortcut("openReview", ["KeyO"]);
    expect(stored()).toEqual({ refresh: ["Mod+KeyK"], openReview: ["KeyO"] });
    store.getState().setShortcut("refresh", null);
    expect(stored()).toEqual({ openReview: ["KeyO"] });
    store.getState().resetShortcuts();
    expect(stored()).toEqual({});
    expect(store.getState().shortcuts).toEqual({});
  });
});
