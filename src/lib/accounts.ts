/**
 * Pure rules for the user's accounts (Settings → Accounts, the sidebar's
 * Providers section): what a row says, the login commands, and the
 * settings patch a switch or the account picker makes.
 */

import type {
  Account,
  AccountChoice,
  AccountState,
  ProviderId,
} from "../ipc/types";
import { PROVIDER_LABEL } from "./reviews";

/** The CLI each platform goes through. */
export const CLI: Record<ProviderId, string> = {
  github: "gh",
  gitlab: "glab",
};

export type AccountTone = "ok" | "warning" | "off";

/** What a row's state says, short, and its tone. */
export function stateLabel(account: Account): {
  text: string;
  tone: AccountTone;
} {
  const state: AccountState = account.state;
  switch (state.kind) {
    case "loggedIn":
      return {
        text: account.user ? `Logged in as ${account.user}` : "Logged in",
        tone: "ok",
      };
    case "notLoggedIn":
      return { text: "Not logged in", tone: "warning" };
    case "error":
      return { text: "Couldn't check", tone: "warning" };
    case "cliMissing":
      return { text: `${CLI[account.provider]} not found`, tone: "warning" };
  }
}

/** The longer explanation under a row, when its state needs one. */
export function stateDetail(account: Account): string | null {
  const state = account.state;
  switch (state.kind) {
    case "loggedIn":
      return null;
    case "notLoggedIn":
      return state.message;
    case "error":
      return state.message;
    case "cliMissing":
      return state.hint;
  }
}

/** Whether the row shows the login command. */
export function needsLogin(account: Account): boolean {
  return account.state.kind !== "loggedIn";
}

/** What logs `provider`'s CLI in to `host` (any host when blank). */
export function loginCommand(provider: ProviderId, host: string): string {
  const h = host.trim();
  return h
    ? `${CLI[provider]} auth login --hostname ${h}`
    : `${CLI[provider]} auth login`;
}

/** `accounts` with `key`'s choice replaced. */
export function withChoice(
  accounts: Record<string, AccountChoice>,
  key: string,
  choice: AccountChoice,
): Record<string, AccountChoice> {
  return { ...accounts, [key]: choice };
}

/** The choice a row stands for (its entry, or the default it shows). */
export function choiceOf(account: Account): AccountChoice {
  return { enabled: account.enabled, account: account.account };
}

/** The providers whose hosts a settings patch of `accounts` changes. */
export function changedProviders(
  before: Record<string, AccountChoice> | undefined,
  after: Record<string, AccountChoice>,
): ProviderId[] {
  const keys = new Set([...Object.keys(before ?? {}), ...Object.keys(after)]);
  const out = new Set<ProviderId>();
  for (const key of keys) {
    const a = before?.[key];
    const b = after[key];
    if (a?.enabled === b?.enabled && a?.account === b?.account) continue;
    const provider = key.split(":")[0];
    if (provider === "github" || provider === "gitlab") out.add(provider);
  }
  return [...out];
}

/** A row's name in the UI: "GitHub · github.com", or the platform alone. */
export function accountName(account: Account): string {
  const label = PROVIDER_LABEL[account.provider];
  return account.host ? `${label} · ${account.host}` : label;
}
