/**
 * What needs attention in the sidebar's Providers section: GitHub unavailable, GitLab not set up,
 * off for an unusable URL or without a token, and the last refresh's errors the status does not
 * already explain.
 */

import type { ProviderId, ProviderStatus, RefreshError } from "../ipc/types";
import { PROVIDER_LABEL } from "./reviews";
import { clockTime } from "./time";

export interface BannerItem {
  key: string;
  /** The provider it is about; its Settings button opens that tab. */
  provider: ProviderId;
  tone: "warning" | "info";
  text: string;
  /** Offer the Settings button. */
  settings?: boolean;
  /** The core's full message, for a tooltip, when the text is reworded. */
  detail?: string;
}

/** The problems, derived from the status and the last refresh. */
export function bannerItems(
  status: ProviderStatus | null,
  errors: readonly RefreshError[],
): BannerItem[] {
  const items: BannerItem[] = [];
  const down = new Set<ProviderId>();
  if (status?.github.enabled && !status.github.installed) {
    down.add("github");
    items.push({
      key: "gh-missing",
      provider: "github",
      tone: "warning",
      text: "GitHub unavailable: the gh CLI was not found. Install it and run gh auth login.",
    });
  } else if (status?.github.enabled && !status.github.loggedIn) {
    down.add("github");
    items.push({
      key: "gh-login",
      provider: "github",
      tone: "warning",
      text: `GitHub unavailable: gh is not logged in for ${status.github.host}. Run gh auth login.`,
      settings: true,
    });
  }
  if (status && !status.gitlab.configured) {
    down.add("gitlab");
    items.push({
      key: "gl-setup",
      provider: "gitlab",
      tone: "info",
      text: "GitLab is not set up.",
      settings: true,
    });
  } else if (status?.gitlab.problem === "invalidUrl") {
    // A hand-edited settings file: saving settings refuses such a URL.
    down.add("gitlab");
    items.push({
      key: "gl-url",
      provider: "gitlab",
      tone: "warning",
      text: "GitLab is off: its URL must start with https:// or http:// and have no user, query or fragment. Fix it in Settings.",
      settings: true,
    });
  } else if (status?.gitlab.configured && !status.gitlab.tokenPresent) {
    down.add("gitlab");
    items.push({
      key: "gl-token",
      provider: "gitlab",
      tone: "warning",
      text: "GitLab has no token: add one in Settings.",
      settings: true,
    });
  }

  // Refresh errors the status above does not already explain.
  const seen = new Set<string>();
  const perReview: Partial<Record<ProviderId, number>> = {};
  for (const e of errors) {
    if (e.unavailable && down.has(e.provider)) continue;
    if (e.review !== null) {
      perReview[e.provider] = (perReview[e.provider] ?? 0) + 1;
      continue;
    }
    const key = `${e.provider}:${e.message}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const until = clockTime(e.rateLimitedUntil);
    if (until) {
      items.push({
        key,
        provider: e.provider,
        tone: "info",
        text: `${PROVIDER_LABEL[e.provider]}: rate limited until ${until}`,
        detail: e.message,
      });
      continue;
    }
    items.push({
      key,
      provider: e.provider,
      tone: "warning",
      text: `${PROVIDER_LABEL[e.provider]}: ${e.message}`,
      settings: e.unavailable,
    });
  }
  for (const p of Object.keys(perReview) as ProviderId[]) {
    const n = perReview[p] ?? 0;
    items.push({
      key: `${p}:reviews`,
      provider: p,
      tone: "warning",
      text: `${PROVIDER_LABEL[p]}: ${n} ${n === 1 ? "review" : "reviews"} could not be refreshed.`,
    });
  }
  return items;
}
