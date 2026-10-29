/**
 * What needs attention in the sidebar's Providers section: an enabled
 * account that is not logged in or could not be checked, a missing CLI, no
 * accounts at all, and the last refresh's errors the accounts do not
 * already explain.
 */

import type {
  Account,
  ProviderId,
  RefreshError,
  RefreshReport,
} from "../ipc/types";
import { CLI } from "./accounts";
import { PROVIDER_LABEL } from "./reviews";
import { clockTime } from "./time";

export interface BannerItem {
  key: string;
  /** The provider it is about. */
  provider: ProviderId;
  tone: "warning" | "info";
  text: string;
  /** Offer the Settings button (Settings → Accounts). */
  settings?: boolean;
  /** The account (`provider:host`) to highlight there. */
  account?: string;
  /** The core's full message, for a tooltip, when the text is reworded. */
  detail?: string;
}

/** When each provider's setup (settings) last changed, in ms since the epoch. */
export type SetupChanges = Partial<Record<ProviderId, number>>;

/**
 * The last refresh's errors that still describe the current setup. A refresh that began before a
 * provider's setup changed ran with the old one: its "Not logged in to <host>" would stay until
 * the next report. Such errors are dropped; the refresh the change started brings its own.
 */
export function currentErrors(
  report: RefreshReport | null,
  changed: SetupChanges,
): RefreshError[] {
  if (!report) return [];
  const began = Date.parse(report.refreshedAt);
  return report.errors.filter((e) => {
    const at = changed[e.provider];
    return at === undefined || !(began < at);
  });
}

/** The problems, derived from the accounts and the last refresh. */
export function bannerItems(
  accounts: readonly Account[] | null,
  errors: readonly RefreshError[],
): BannerItem[] {
  const items: BannerItem[] = [];
  /** `provider:host` of the enabled accounts that cannot be used. */
  const down = new Set<string>();
  const missing = new Set<ProviderId>();
  for (const a of accounts ?? []) {
    const label = PROVIDER_LABEL[a.provider];
    const cli = CLI[a.provider];
    switch (a.state.kind) {
      case "loggedIn":
        break;
      case "cliMissing":
        // Only for a provider the user set up before.
        if (!a.enabled) break;
        missing.add(a.provider);
        items.push({
          key: `missing:${a.provider}`,
          provider: a.provider,
          tone: "warning",
          text: `${label} unavailable: the ${cli} CLI was not found. Install it and run ${cli} auth login.`,
          settings: true,
          account: a.key,
        });
        break;
      case "notLoggedIn":
        if (!a.enabled) break;
        down.add(a.key);
        items.push({
          key: `login:${a.key}`,
          provider: a.provider,
          tone: "warning",
          text: `${label} unavailable: ${cli} is not logged in to ${a.host}. Run ${a.loginCommand}.`,
          settings: true,
          account: a.key,
          detail: a.state.message ?? undefined,
        });
        break;
      case "error":
        if (!a.enabled) break;
        items.push({
          key: `error:${a.key}`,
          provider: a.provider,
          tone: "warning",
          text: a.host
            ? `${label} (${a.host}): ${a.state.message}`
            : `${label}: ${a.state.message}`,
          settings: !a.host,
          account: a.key,
        });
        break;
    }
  }
  // No host at all: say how to start.
  if (accounts && !accounts.some((a) => a.host) && missing.size < 2) {
    items.push({
      key: "no-accounts",
      provider: "github",
      tone: "info",
      text: "No accounts yet: log in with gh auth login or glab auth login.",
      settings: true,
    });
  }

  // Refresh errors the accounts above do not already explain.
  const seen = new Set<string>();
  const perReview: Partial<Record<ProviderId, number>> = {};
  for (const e of errors) {
    if (
      e.unavailable &&
      (missing.has(e.provider) || down.has(`${e.provider}:${e.host}`))
    )
      continue;
    if (e.review !== null) {
      perReview[e.provider] = (perReview[e.provider] ?? 0) + 1;
      continue;
    }
    const key = `${e.provider}:${e.host}:${e.message}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const until = clockTime(e.rateLimitedUntil);
    if (until) {
      items.push({
        key,
        provider: e.provider,
        tone: "info",
        text: `${PROVIDER_LABEL[e.provider]} (${e.host}): rate limited until ${until}`,
        detail: e.message,
      });
      continue;
    }
    items.push({
      key,
      provider: e.provider,
      tone: "warning",
      text: `${PROVIDER_LABEL[e.provider]} (${e.host}): ${e.message}`,
      settings: e.unavailable,
      account: `${e.provider}:${e.host}`,
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
