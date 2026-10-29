/** One line per account for the sidebar's Providers section. */

import type { Account } from "../ipc/types";
import { stateLabel, type AccountTone } from "./accounts";
import { PROVIDER_LABEL } from "./reviews";

export interface ProviderLine {
  /** The account's key (`provider:host`). */
  key: string;
  label: string;
  /** The host it talks to, if any. */
  host: string;
  state: string;
  tone: AccountTone;
}

/**
 * The accounts Argus polls (or would, but cannot): every enabled host, and
 * a missing CLI. Hosts switched off are left out.
 */
export function providerLines(accounts: readonly Account[]): ProviderLine[] {
  return accounts
    .filter((a) => a.enabled)
    .map((a) => {
      const { text, tone } = stateLabel(a);
      return {
        key: a.key,
        label: PROVIDER_LABEL[a.provider],
        host: a.host,
        // The sidebar is narrow: the user is in Settings.
        state: a.state.kind === "loggedIn" ? "Connected" : text,
        tone,
      };
    });
}
