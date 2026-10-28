/** The live keymap: the defaults with the user's changes from Settings. */

import { useMemo } from "react";

import { useSettingsStore } from "../state/settingsStore";
import { resolveKeymap, type Keymap } from "./keymap";

export function useKeymap(): Keymap {
  const overrides = useSettingsStore((s) => s.shortcuts);
  return useMemo(() => resolveKeymap(overrides), [overrides]);
}

/** The keymap right now, for event handlers outside React's render. */
export function currentKeymap(): Keymap {
  return resolveKeymap(useSettingsStore.getState().shortcuts);
}
