/**
 * Opens a review link in the OS default browser through
 * `tauri-plugin-opener`. A bare `<a target="_blank">` resolves to a
 * new-window request the webview has no handler for, so it would do nothing.
 *
 * Comment links may be relative to the review's page (GitLab's `/uploads/…`
 * attachments and `#note_…` anchors, GitHub's `../blob/…`): `resolveLink`
 * resolves them against the review's web URL. Only http(s) ever opens.
 */

import { openUrl } from "@tauri-apps/plugin-opener";

import type { ProviderId } from "../ipc/types";
import { logWarning } from "./log";

export function openExternal(url: string): void {
  if (!/^https?:\/\//i.test(url)) return;
  void openUrl(url).catch((e: unknown) =>
    logWarning("open link", e instanceof Error ? e.message : String(e)),
  );
}

/** The review page a comment's relative links are relative to. */
export interface LinkBase {
  url: string;
  provider: ProviderId;
}

const httpOnly = (url: string): string | null =>
  /^https?:\/\//i.test(url) ? url : null;

/**
 * `href` as an http(s) URL to open: as is when absolute, else against the
 * review's page (GitLab uploads against its project); null when nothing
 * http(s) comes out (`mailto:`, no review, unparseable).
 */
export function resolveLink(
  href: string,
  base: LinkBase | null,
): string | null {
  if (/^[a-z][a-z0-9+.-]*:/i.test(href)) return httpOnly(href);
  if (!base || href === "") return null;
  try {
    if (base.provider === "gitlab" && href.startsWith("/uploads/")) {
      // GitLab serves a project's attachments under the project, not the
      // instance root: `https://host/group/project/uploads/…`.
      const project = base.url.split("/-/")[0].replace(/\/$/, "");
      return httpOnly(`${project}${href}`);
    }
    return httpOnly(new URL(href, base.url).toString());
  } catch {
    return null;
  }
}
