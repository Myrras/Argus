/**
 * Settings: GitHub host and account, GitLab URL and token, poll interval,
 * AI on/off. The token field is write-only: it goes to `set_gitlab_token`
 * on save and is never part of the `Settings` sent to `update_settings`;
 * the dialog only ever shows whether one is stored.
 */

import { useEffect, useState } from "react";
import type { FormEvent } from "react";

import type { Settings } from "../ipc/types";
import { type TokenChange, useSettingsStore } from "../state/settingsStore";
import { Modal } from "./Modal";
import styles from "./SettingsDialog.module.css";

interface Draft {
  githubEnabled: boolean;
  githubHost: string;
  githubAccount: string;
  gitlabUrl: string;
  pollMinutes: string;
  aiEnabled: boolean;
}

function toDraft(s: Settings): Draft {
  return {
    githubEnabled: s.github.enabled,
    githubHost: s.github.host,
    githubAccount: s.github.account ?? "",
    gitlabUrl: s.gitlab.baseUrl,
    pollMinutes: String(Math.max(1, Math.round(s.pollIntervalSecs / 60))),
    aiEnabled: s.ai.enabled,
  };
}

/** The draft applied over the stored settings (unshown fields kept). */
function fromDraft(base: Settings, d: Draft): Settings {
  const minutes = Math.max(1, Math.round(Number(d.pollMinutes) || 1));
  return {
    ...base,
    github: {
      ...base.github,
      enabled: d.githubEnabled,
      host: d.githubHost.trim(),
      account: d.githubAccount.trim() || null,
    },
    gitlab: { ...base.gitlab, baseUrl: d.gitlabUrl.trim() },
    pollIntervalSecs: minutes * 60,
    ai: { ...base.ai, enabled: d.aiEnabled },
  };
}

export function SettingsDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const settings = useSettingsStore((s) => s.settings);
  const tokenPresent = useSettingsStore((s) => s.tokenPresent);
  const saving = useSettingsStore((s) => s.saving);
  const lastError = useSettingsStore((s) => s.lastError);
  const save = useSettingsStore((s) => s.save);

  const [draft, setDraft] = useState<Draft | null>(null);
  const [token, setToken] = useState("");
  const [clearToken, setClearToken] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);

  // A fresh draft each time the dialog opens; the token field always starts
  // empty (the stored token is never read back).
  useEffect(() => {
    if (open && settings) {
      setDraft(toDraft(settings));
      setToken("");
      setClearToken(false);
      setLocalError(null);
    }
  }, [open, settings]);

  if (!open || !settings || !draft) {
    return null;
  }

  const update = <K extends keyof Draft>(key: K, value: Draft[K]) =>
    setDraft({ ...draft, [key]: value });

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    if (!settings || !draft) return;
    setLocalError(null);
    const next = fromDraft(settings, draft);
    const trimmed = token.trim();
    if (trimmed && !next.gitlab.baseUrl) {
      setLocalError("Set the GitLab URL before its token.");
      return;
    }
    const change: TokenChange = trimmed
      ? { action: "set", token: trimmed }
      : clearToken
        ? { action: "clear" }
        : { action: "keep" };
    if (await save(next, change)) {
      setToken("");
      onClose();
    }
  }

  const error = localError ?? lastError?.message ?? null;
  const tokenState = clearToken
    ? "Will be removed"
    : tokenPresent
      ? "Token set"
      : "Not set";

  return (
    <Modal
      open={open}
      title="Settings"
      onClose={onClose}
      width={440}
      footer={
        <>
          {error && (
            <span className={styles.error} role="alert">
              {error}
            </span>
          )}
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="submit"
            form="settings-form"
            className="primary"
            disabled={saving}
          >
            {saving ? "Saving…" : "Save"}
          </button>
        </>
      }
    >
      <form id="settings-form" className={styles.form} onSubmit={onSubmit}>
        <fieldset className={styles.section}>
          <legend className={styles.legend}>GitHub</legend>
          <label className={styles.check}>
            <input
              type="checkbox"
              checked={draft.githubEnabled}
              onChange={(e) => update("githubEnabled", e.target.checked)}
            />
            Use GitHub (through the gh CLI)
          </label>
          <div className={styles.grid}>
            <div>
              <label htmlFor="gh-host">Host</label>
              <input
                id="gh-host"
                value={draft.githubHost}
                placeholder="github.com"
                spellCheck={false}
                onChange={(e) => update("githubHost", e.target.value)}
              />
            </div>
            <div>
              <label htmlFor="gh-account">Account (optional)</label>
              <input
                id="gh-account"
                value={draft.githubAccount}
                placeholder="gh's active account"
                spellCheck={false}
                onChange={(e) => update("githubAccount", e.target.value)}
              />
            </div>
          </div>
        </fieldset>

        <fieldset className={styles.section}>
          <legend className={styles.legend}>GitLab</legend>
          <div>
            <label htmlFor="gl-url">Base URL</label>
            <input
              id="gl-url"
              value={draft.gitlabUrl}
              placeholder="https://gitlab.example.com (empty = off)"
              spellCheck={false}
              onChange={(e) => update("gitlabUrl", e.target.value)}
            />
          </div>
          <div>
            <div className={styles.labelRow}>
              <label htmlFor="gl-token">Personal access token</label>
              <span
                className={styles.tokenState}
                data-set={tokenPresent && !clearToken}
              >
                {tokenState}
              </span>
            </div>
            <div className={styles.tokenRow}>
              <input
                id="gl-token"
                type="password"
                autoComplete="off"
                value={token}
                placeholder={
                  tokenPresent
                    ? "Leave empty to keep the stored token"
                    : "glpat-…"
                }
                onChange={(e) => {
                  setToken(e.target.value);
                  if (e.target.value) setClearToken(false);
                }}
              />
              {tokenPresent && (
                <button
                  type="button"
                  className="ghost"
                  aria-pressed={clearToken}
                  onClick={() => {
                    setClearToken(!clearToken);
                    setToken("");
                  }}
                >
                  {clearToken ? "Keep" : "Remove"}
                </button>
              )}
            </div>
            <p className={styles.hint}>Kept in the system keychain.</p>
          </div>
        </fieldset>

        <fieldset className={styles.section}>
          <legend className={styles.legend}>General</legend>
          <div className={styles.inline}>
            <label htmlFor="poll">Refresh every</label>
            <input
              id="poll"
              type="number"
              min={1}
              max={120}
              step={1}
              className={styles.number}
              value={draft.pollMinutes}
              onChange={(e) => update("pollMinutes", e.target.value)}
            />
            <span className={styles.unit}>minutes</span>
          </div>
          <label className={styles.check}>
            <input
              type="checkbox"
              checked={draft.aiEnabled}
              onChange={(e) => update("aiEnabled", e.target.checked)}
            />
            AI analysis
            <span className={styles.hint}>
              (saved now, used in a later release)
            </span>
          </label>
        </fieldset>
      </form>
    </Modal>
  );
}
