/**
 * Opens a review link in the OS default browser through
 * `tauri-plugin-opener`. A bare `<a target="_blank">` resolves to a
 * new-window request the webview has no handler for, so it would do nothing.
 */

import { openUrl } from "@tauri-apps/plugin-opener";

import { logWarning } from "./log";

export function openExternal(url: string): void {
  if (!/^https?:\/\//i.test(url)) return;
  void openUrl(url).catch((e: unknown) =>
    logWarning("open link", e instanceof Error ? e.message : String(e)),
  );
}
