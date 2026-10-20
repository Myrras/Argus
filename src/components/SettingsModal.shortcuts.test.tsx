import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve(null)),
}));
vi.mock("../lib/platform", () => ({ isMac: true, isWindows: false }));

import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { useSettingsStore } from "../state/settingsStore";
import { SettingsModal } from "./SettingsModal";

const stored = () =>
  JSON.parse(localStorage.getItem("argus.settings") ?? "{}").shortcuts;

const onClose = vi.fn();

function renderTab() {
  render(<SettingsModal open onClose={onClose} />);
  fireEvent.click(screen.getByRole("tab", { name: "Shortcuts" }));
}

const row = (label: string) => screen.getByRole("group", { name: label });
const keys = (label: string) =>
  Array.from(row(label).querySelectorAll("kbd")).map((k) => k.textContent);
const record = (label: string) =>
  fireEvent.click(
    within(row(label)).getByRole("button", { name: /^Record shortcut/ }),
  );
/** A key press while recording (the recorder listens on `window`). */
function press(init: KeyboardEventInit & { key: string }) {
  const event = new KeyboardEvent("keydown", {
    bubbles: true,
    cancelable: true,
    ...init,
  });
  act(() => {
    window.dispatchEvent(event);
  });
  return event;
}

beforeEach(() => {
  localStorage.clear();
  useSettingsStore.getState().resetSettings();
  useSettingsRequestStore.getState().clear();
});

describe("SettingsModal — Shortcuts tab", () => {
  it("lists the actions with their default bindings", () => {
    renderTab();
    expect(keys("Next review")).toEqual(["↓"]);
    expect(keys("Previous review")).toEqual(["↑"]);
    expect(keys("Open review")).toEqual(["↩"]);
    expect(keys("Refresh")).toEqual(["⇧R", "⌘R"]);
    expect(keys("Needs review")).toEqual(["1"]);
    expect(keys("Closed without my approval")).toEqual(["4"]);
    // The native menu owns ⌘, on macOS.
    expect(screen.queryByRole("group", { name: "Open Settings" })).toBeNull();
  });

  it("opens from a settings request", () => {
    useSettingsRequestStore.getState().open("shortcuts");
    render(<SettingsModal open onClose={() => undefined} />);
    expect(screen.getByRole("tab", { name: "Shortcuts" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("records a new binding and stores only that change", () => {
    renderTab();
    record("Refresh");
    expect(within(row("Refresh")).getByText("Press keys…")).toBeVisible();
    // A modifier on its own keeps waiting.
    press({ key: "Meta", metaKey: true });
    expect(within(row("Refresh")).getByText("Press keys…")).toBeVisible();
    const e = press({ key: "u", code: "KeyU", metaKey: true });
    expect(e.defaultPrevented).toBe(true);
    expect(keys("Refresh")).toEqual(["⌘U"]);
    expect(stored()).toEqual({ refresh: ["Mod+KeyU"] });
  });

  it("takes a bare key, unlike Iris (Argus has no type-to-select)", () => {
    renderTab();
    record("Open review");
    // A key no action uses (O opens the file in the editor).
    press({ key: "y", code: "KeyY" });
    expect(keys("Open review")).toEqual(["Y"]);
  });

  it("records Option combinations by the physical key", () => {
    renderTab();
    record("Refresh");
    press({ key: "ø", code: "KeyO", metaKey: true, altKey: true });
    expect(keys("Refresh")).toEqual(["⌥⌘O"]);
  });

  it("cancels with Escape without closing the dialog", () => {
    renderTab();
    record("Refresh");
    onClose.mockClear();
    press({ key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "Settings" })).toBeVisible();
    expect(within(row("Refresh")).queryByText("Press keys…")).toBeNull();
    expect(keys("Refresh")).toEqual(["⇧R", "⌘R"]);
  });

  it("refuses a key another action uses, and offers a swap", () => {
    renderTab();
    record("Open review");
    press({ key: "1", code: "Digit1" });
    expect(within(row("Open review")).getByRole("alert")).toHaveTextContent(
      "1 is already used by Needs review.",
    );
    expect(useSettingsStore.getState().shortcuts).toEqual({});

    fireEvent.click(screen.getByRole("button", { name: "Swap them" }));
    expect(keys("Open review")).toEqual(["1"]);
    expect(keys("Needs review")).toEqual(["↩"]);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("refuses an alias of another action too", () => {
    renderTab();
    record("Next review");
    press({ key: "r", code: "KeyR", metaKey: true });
    expect(screen.getByRole("alert")).toHaveTextContent(
      "⌘R is already used by Refresh.",
    );
  });

  it("refuses fixed keys and keeps recording", () => {
    renderTab();
    record("Refresh");
    press({ key: "a", code: "KeyA", metaKey: true });
    expect(screen.getByRole("alert")).toHaveTextContent(
      "⌘A is fixed (Select all).",
    );
    press({ key: "Tab", code: "Tab" });
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Tab is fixed (Move the focus).",
    );
    // Still recording: the next valid press is taken.
    press({ key: "j", code: "KeyJ", metaKey: true });
    expect(keys("Refresh")).toEqual(["⌘J"]);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("resets a row and everything", () => {
    renderTab();
    const reset = (label: string) =>
      within(row(label)).getByRole("button", { name: `Reset ${label}` });
    expect(reset("Refresh")).toBeDisabled();
    expect(screen.getByRole("button", { name: "Reset all" })).toBeDisabled();

    record("Refresh");
    press({ key: "j", code: "KeyJ", metaKey: true });
    record("Open review");
    press({ key: "y", code: "KeyY" });
    expect(screen.getByRole("button", { name: "Reset all" })).toBeEnabled();

    fireEvent.click(reset("Refresh"));
    expect(keys("Refresh")).toEqual(["⇧R", "⌘R"]);
    expect(stored()).toEqual({ openReview: ["KeyY"] });

    fireEvent.click(screen.getByRole("button", { name: "Reset all" }));
    expect(keys("Open review")).toEqual(["↩"]);
    expect(stored()).toEqual({});
  });

  it("keeps the shortcuts after a reload", async () => {
    renderTab();
    record("Refresh");
    press({ key: "j", code: "KeyJ", metaKey: true });
    vi.resetModules();
    const fresh = (await import("../state/settingsStore")).useSettingsStore;
    expect(fresh.getState().shortcuts).toEqual({ refresh: ["Mod+KeyJ"] });
  });
});
