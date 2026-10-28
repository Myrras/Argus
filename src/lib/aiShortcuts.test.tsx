import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./platform", () => ({ isMac: false, isWindows: false }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
}));

import { invoke } from "@tauri-apps/api/core";

import { useAiStore } from "../state/aiStore";
import { useLayoutStore } from "../state/layoutStore";
import { useReviewStore } from "../state/reviewStore";
import { useSettingsStore } from "../state/settingsStore";
import { aiRun, finding } from "../test/aiFixtures";
import { review } from "../test/fixtures";
import { DEFAULT_KEYMAP } from "./keymap";
import { useAppShortcuts } from "./useAppShortcuts";

const r = review();
const f = finding({ reviewId: r.id });
const initialAi = useAiStore.getState();
const initialReviews = useReviewStore.getState();

function press(key: string, code: string) {
  const event = new KeyboardEvent("keydown", {
    key,
    code,
    bubbles: true,
    cancelable: true,
  });
  act(() => {
    window.dispatchEvent(event);
  });
  return event;
}

function seed(enabled: boolean) {
  useAiStore.setState(
    {
      ...initialAi,
      runs: { [r.id]: aiRun({ reviewId: r.id, enabled }) },
      byReview: {
        [r.id]: {
          findings: [f],
          checklist: [],
          verdicts: [],
          loading: false,
          error: null,
        },
      },
      ui: {},
      focused: null,
    },
    true,
  );
}

beforeEach(() => {
  localStorage.clear();
  vi.mocked(invoke).mockClear();
  useSettingsStore.getState().resetShortcuts();
  useReviewStore.setState(
    { ...initialReviews, reviews: [r], selectedId: r.id, loaded: true },
    true,
  );
  useLayoutStore.setState({ aiPanelOpen: true });
  seed(true);
});

describe("AI shortcuts", () => {
  it("are a and x by default", () => {
    expect(DEFAULT_KEYMAP.toggleFindings).toEqual(["KeyA"]);
    expect(DEFAULT_KEYMAP.toggleFindingSelected).toEqual(["KeyX"]);
  });

  it("a toggles the AI panel, only with the AI pass on", () => {
    renderHook(() => useAppShortcuts());
    expect(press("a", "KeyA").defaultPrevented).toBe(true);
    expect(useLayoutStore.getState().aiPanelOpen).toBe(false);
    press("a", "KeyA");
    expect(useLayoutStore.getState().aiPanelOpen).toBe(true);
    seed(false);
    expect(press("a", "KeyA").defaultPrevented).toBe(false);
    expect(useLayoutStore.getState().aiPanelOpen).toBe(true);
  });

  it("x selects the finding last clicked, and never posts", () => {
    renderHook(() => useAppShortcuts());
    // Nothing focused: the key is left alone.
    expect(press("x", "KeyX").defaultPrevented).toBe(false);
    useAiStore.getState().focus(f.id);
    expect(press("x", "KeyX").defaultPrevented).toBe(true);
    expect(useAiStore.getState().ui[f.id]?.selected).toBe(true);
    press("x", "KeyX");
    expect(useAiStore.getState().ui[f.id]?.selected).toBe(false);
    expect(vi.mocked(invoke)).not.toHaveBeenCalled();
  });
});
