/**
 * Settings → Accounts: the hosts the user's `gh` and `glab` are logged in
 * to, one card each (the core's `list_accounts`, through
 * `backendSettingsStore.accounts`). Argus has no host settings of its own:
 * a host appears once the CLI is logged in to it, and is on unless switched
 * off here (`Settings.accounts`, keyed `provider:host`).
 *
 * Per card: the platform, the host, the login's state, an on/off switch
 * (applies at once) and, for a GitHub host with several gh accounts, which
 * one Argus uses. A host that is not logged in (or a CLI that is missing)
 * shows the command that fixes it, copyable. Below: "Add an account" with
 * the login commands for a new host and Check again (`list_accounts` with
 * `recheck`).
 *
 * Opened on an account (`settingsRequestStore.open("accounts", key)`, from
 * the sidebar's Providers section), the tab scrolls its card into view and
 * highlights it.
 */

import { useEffect, useRef, useState } from "react";

import type { Account } from "../ipc/types";
import {
  choiceOf,
  loginCommand,
  needsLogin,
  stateDetail,
  stateLabel,
} from "../lib/accounts";
import { PROVIDER_LABEL } from "../lib/reviews";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import own from "./AccountsTab.module.css";
import { PlatformBadge } from "./Badge";
import { CodeBlock } from "./Markdown";
import styles from "./SettingsModal.module.css";

export function AccountsTab() {
  const accounts = useBackendSettingsStore((s) => s.accounts);
  const requested = useSettingsRequestStore((s) => s.account);
  const seq = useSettingsRequestStore((s) => s.seq);
  const cards = useRef(new Map<string, HTMLElement>());
  const [highlighted, setHighlighted] = useState<string | null>(null);

  useEffect(() => {
    if (requested) setHighlighted(requested);
  }, [requested, seq]);
  useEffect(() => {
    if (!highlighted || !accounts) return;
    cards.current
      .get(highlighted)
      ?.scrollIntoView?.({ block: "nearest", behavior: "smooth" });
  }, [highlighted, accounts, seq]);

  return (
    <>
      <section className={styles.section}>
        <h3 className={styles.sectionLabel}>Accounts</h3>
        <small className={styles.rowHelp}>
          Argus uses your gh and glab logins; it never sees or stores a token. A
          host appears here once you log in to it with the CLI, and is used
          until you switch it off.
        </small>
        {accounts === null && <p className={styles.empty}>Checking…</p>}
        {accounts !== null && accounts.length === 0 && (
          <p className={styles.empty}>
            No accounts yet. Log in with gh or glab below, then Check again.
          </p>
        )}
        {accounts?.map((account) => (
          <AccountCard
            key={account.key}
            account={account}
            highlighted={highlighted === account.key}
            cardRef={(el) => {
              if (el) cards.current.set(account.key, el);
              else cards.current.delete(account.key);
            }}
          />
        ))}
      </section>
      <AddAccount />
    </>
  );
}

function AccountCard({
  account,
  highlighted,
  cardRef,
}: {
  account: Account;
  highlighted: boolean;
  cardRef: (el: HTMLElement | null) => void;
}) {
  const setAccount = useBackendSettingsStore((s) => s.setAccount);
  const ready = useBackendSettingsStore((s) => s.settings !== null);
  const { text, tone } = stateLabel(account);
  const detail = stateDetail(account);
  const label = PROVIDER_LABEL[account.provider];
  const name = account.host || label;
  // A missing CLI or one that did not answer has no host to switch.
  const switchable = account.host !== "";
  const picker = account.provider === "github" && account.accounts.length > 1;

  return (
    <div
      ref={cardRef}
      className={`${styles.card} ${own.card}`}
      role="group"
      aria-label={`${label} ${name}`}
      data-slot="account-card"
      data-enabled={account.enabled ? "true" : "false"}
      data-highlighted={highlighted ? "true" : undefined}
    >
      <div className={own.header}>
        <PlatformBadge provider={account.provider} />
        <span className={`${own.host} mono`} title={name}>
          {name}
        </span>
        <span
          className={own.state}
          data-tone={account.enabled || !switchable ? tone : "off"}
          title={account.user ?? undefined}
        >
          {switchable && !account.enabled ? "Off" : text}
        </span>
        {switchable && (
          <button
            type="button"
            role="switch"
            aria-checked={account.enabled}
            aria-label={`Use ${label} at ${account.host}`}
            className={`${styles.toggle} ${own.switch}`}
            data-on={account.enabled ? "true" : "false"}
            disabled={!ready}
            onClick={() =>
              void setAccount(account.key, {
                ...choiceOf(account),
                enabled: !account.enabled,
              })
            }
          />
        )}
      </div>
      {picker && (
        <div className={own.picker}>
          <label htmlFor={`account-${account.key}`}>gh account</label>
          <select
            id={`account-${account.key}`}
            value={account.account ?? ""}
            disabled={!ready}
            onChange={(e) =>
              void setAccount(account.key, {
                ...choiceOf(account),
                account: e.target.value || null,
              })
            }
          >
            <option value="">
              {account.active
                ? `gh's active account (${account.active})`
                : "gh's active account"}
            </option>
            {account.accounts.map((login) => (
              <option key={login} value={login}>
                {login}
              </option>
            ))}
          </select>
        </div>
      )}
      {detail && <small className={styles.rowHelp}>{detail}</small>}
      {needsLogin(account) && account.state.kind !== "error" && (
        <>
          <small className={styles.rowHelp}>
            {account.state.kind === "cliMissing"
              ? "Then log in, in a terminal:"
              : "Log in once, in a terminal, then Check again:"}
          </small>
          <CodeBlock info="sh" code={account.loginCommand} />
        </>
      )}
    </div>
  );
}

function AddAccount() {
  const checkAccounts = useBackendSettingsStore((s) => s.checkAccounts);
  const [host, setHost] = useState("");
  const [checking, setChecking] = useState(false);
  const check = async () => {
    setChecking(true);
    await checkAccounts();
    setChecking(false);
  };
  return (
    <section className={styles.section} aria-label="Add an account">
      <h3 className={styles.sectionLabel}>Add an account</h3>
      <small className={styles.rowHelp}>
        Log the CLI in to the host in a terminal, then Check again. GitHub
        Enterprise and self-hosted GitLab work the same way.
      </small>
      <div className={styles.field}>
        <label htmlFor="account-host">Host (optional)</label>
        <input
          id="account-host"
          type="text"
          value={host}
          placeholder="github.com, gitlab.example.com"
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          onChange={(e) => setHost(e.target.value)}
        />
      </div>
      <span className={own.cli}>GitHub (gh)</span>
      <CodeBlock info="sh" code={loginCommand("github", host)} />
      <span className={own.cli}>GitLab (glab)</span>
      <CodeBlock info="sh" code={loginCommand("gitlab", host)} />
      <div className={styles.editActions}>
        <button type="button" disabled={checking} onClick={() => void check()}>
          {checking ? "Checking…" : "Check again"}
        </button>
      </div>
    </section>
  );
}
