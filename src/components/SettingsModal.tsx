/**
 * App settings dialog, tabbed like Iris's and Podium's.
 *
 * - General: theme and reduced motion (`themeStore` / `settingsStore`), and
 *   the refresh interval (the core's `settings.json`).
 * - GitHub: on/off, host and an optional `gh` account.
 * - GitLab: the instance URL and the personal access token. The token is
 *   write-only: it goes to `set_gitlab_token` (`backendSettingsStore`
 *   `.setGitlabToken`) and is never part of the `Settings` sent to
 *   `update_settings`; the dialog only shows whether one is stored.
 * - AI: analysis on/off, and the local clone per repository (cards in Iris's
 *   Editors look, the folder picked with the native dialog).
 * - Shortcuts: rebind the keyboard shortcuts (`ShortcutsTab`, `lib/keymap`).
 *
 * The family's save model: switches and steppers apply at once; text fields
 * sit in a form with its own Save. Backend failures are error toasts, input
 * the dialog can check itself is refused inline.
 *
 * A new tab: add the id to `SettingsTabId`, an entry to `TABS` and a branch
 * in `TabBody`.
 */

import { useEffect, useState } from "react";

import { toIpcError } from "../ipc/commands";
import {
  cloneError,
  normalizeRepo,
  withClone,
  withoutClone,
} from "../lib/clonePaths";
import { NOT_READ_ONLY_NOTE, useReadOnlyGuaranteed } from "../state/aiStore";
import {
  POLL_MINUTES_MAX,
  POLL_MINUTES_MIN,
  useBackendSettingsStore,
} from "../state/backendSettingsStore";
import {
  useSettingsRequestStore,
  type SettingsTabId,
} from "../state/settingsRequestStore";
import { useSettingsStore } from "../state/settingsStore";
import { useThemeStore, type ThemeMode } from "../state/themeStore";
import { toastSuccess } from "../state/toastStore";
import { AddIcon, CheckIcon, MinusIcon } from "./icons";
import { Modal } from "./Modal";
import { ShortcutsTab } from "./ShortcutsTab";
import styles from "./SettingsModal.module.css";

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

const TABS: [SettingsTabId, string][] = [
  ["general", "General"],
  ["github", "GitHub"],
  ["gitlab", "GitLab"],
  ["ai", "AI"],
  ["shortcuts", "Shortcuts"],
];

/**
 * Each theme's background / accent / text, as fixed preview tokens
 * (`--swatch-*` in tokens.css) — the picker shows every theme at once, so it
 * cannot use the live palette tokens.
 */
const THEMES: {
  mode: ThemeMode;
  label: string;
  swatches: [string, string, string];
}[] = [
  {
    mode: "dark",
    label: "Dark",
    swatches: [
      "var(--swatch-dark-bg)",
      "var(--swatch-dark-accent)",
      "var(--swatch-dark-text)",
    ],
  },
  {
    mode: "light",
    label: "Light",
    swatches: [
      "var(--swatch-light-bg)",
      "var(--swatch-light-accent)",
      "var(--swatch-light-text)",
    ],
  },
  {
    mode: "retro",
    label: "Retro",
    swatches: [
      "var(--swatch-retro-bg)",
      "var(--swatch-retro-accent)",
      "var(--swatch-retro-text)",
    ],
  },
];

/** Checkbox toggle row with label and optional helper text. */
function SettingToggle({
  label,
  help,
  value,
  disabled,
  onChange,
}: {
  label: string;
  help?: string;
  value: boolean;
  disabled?: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className={styles.row}>
      <span className={styles.rowLabel}>
        {label}
        {help && <small className={styles.rowHelp}>{help}</small>}
      </span>
      <button
        type="button"
        role="switch"
        aria-checked={value}
        className={styles.toggle}
        data-on={value ? "true" : "false"}
        disabled={disabled}
        onClick={() => onChange(!value)}
        aria-label={label}
      />
    </div>
  );
}

export function SettingsModal({ open, onClose }: SettingsModalProps) {
  const [tab, setTab] = useState<SettingsTabId>("general");
  const requestedTab = useSettingsRequestStore((s) => s.request);
  useEffect(() => {
    if (open && requestedTab) setTab(requestedTab);
  }, [open, requestedTab]);
  // Fresh from the core every time the dialog opens.
  useEffect(() => {
    if (open) void useBackendSettingsStore.getState().load();
  }, [open]);

  return (
    <Modal open={open} title="Settings" onClose={onClose} width={520}>
      {/* Tabs */}
      <div className={styles.tabs} role="tablist" aria-label="Settings tabs">
        {TABS.map(([id, label]) => (
          <button
            key={id}
            type="button"
            role="tab"
            aria-selected={tab === id}
            className={styles.tab}
            data-active={tab === id ? "true" : "false"}
            onClick={() => setTab(id)}
          >
            {label}
          </button>
        ))}
      </div>

      <TabBody tab={tab} />
    </Modal>
  );
}

function TabBody({ tab }: { tab: SettingsTabId }) {
  switch (tab) {
    case "general":
      return <GeneralTab />;
    case "github":
      return <GithubTab />;
    case "gitlab":
      return <GitlabTab />;
    case "ai":
      return (
        <>
          <AiTab />
          <ClonesSection />
        </>
      );
    case "shortcuts":
      return <ShortcutsTab />;
  }
}

/** A − value + control for a bounded whole number (`label` names it). */
function Stepper({
  label,
  value,
  min,
  max,
  show = String,
  onChange,
}: {
  label: string;
  value: number | null;
  min: number;
  max: number;
  show?: (value: number) => string;
  onChange: (value: number) => void;
}) {
  return (
    <div className={styles.stepper}>
      <button
        type="button"
        aria-label={`Decrease ${label.toLowerCase()}`}
        disabled={value === null || value <= min}
        onClick={() => value !== null && onChange(value - 1)}
      >
        <MinusIcon size={14} />
      </button>
      <span className={styles.stepperValue} aria-label={label}>
        {value === null ? "–" : show(value)}
      </span>
      <button
        type="button"
        aria-label={`Increase ${label.toLowerCase()}`}
        disabled={value === null || value >= max}
        onClick={() => value !== null && onChange(value + 1)}
      >
        <AddIcon size={14} />
      </button>
    </div>
  );
}

function GeneralTab() {
  const mode = useThemeStore((s) => s.mode);
  const setTheme = useThemeStore((s) => s.setTheme);
  const reduceMotion = useSettingsStore((s) => s.appearance.reduceMotion);
  const setPref = useSettingsStore((s) => s.set);
  const settings = useBackendSettingsStore((s) => s.settings);
  const save = useBackendSettingsStore((s) => s.save);
  const minutes = settings
    ? Math.min(
        POLL_MINUTES_MAX,
        Math.max(POLL_MINUTES_MIN, Math.round(settings.pollIntervalSecs / 60)),
      )
    : null;

  return (
    <>
      {/* Appearance */}
      <section className={styles.section}>
        <h3 className={styles.sectionLabel}>Appearance</h3>
        <div className={styles.themeOptions}>
          {THEMES.map(({ mode: m, label, swatches }) => (
            <button
              key={m}
              type="button"
              className={`${styles.themeOption} ${mode === m ? styles.active : ""}`}
              onClick={() => setTheme(m)}
              aria-pressed={mode === m}
            >
              <span className={styles.swatch}>
                {swatches.map((color, i) => (
                  <span
                    key={i}
                    className={styles.swatchSegment}
                    style={{ background: color }}
                  />
                ))}
              </span>
              <span className={styles.themeLabel}>{label}</span>
              {mode === m && (
                <span className={styles.check} aria-hidden>
                  <CheckIcon />
                </span>
              )}
            </button>
          ))}
        </div>
        <SettingToggle
          label="Reduce motion"
          help="Disable app animations. Your OS reduced-motion preference still applies regardless."
          value={reduceMotion}
          onChange={(v) => setPref("appearance", { reduceMotion: v })}
        />
      </section>

      {/* Refresh */}
      <section className={styles.section}>
        <h3 className={styles.sectionLabel}>Refresh</h3>
        <div className={styles.row}>
          <span className={styles.rowLabel}>
            Refresh every
            <small className={styles.rowHelp}>
              How often Argus checks GitHub and GitLab in the background. r
              refreshes at once.
            </small>
          </span>
          <Stepper
            label="Refresh every"
            value={minutes}
            min={POLL_MINUTES_MIN}
            max={POLL_MINUTES_MAX}
            show={(n) => `${n} min`}
            onChange={(n) => void save({ pollIntervalSecs: n * 60 })}
          />
        </div>
      </section>
    </>
  );
}

function GithubTab() {
  const settings = useBackendSettingsStore((s) => s.settings);
  const save = useBackendSettingsStore((s) => s.save);
  const [host, setHost] = useState("");
  const [account, setAccount] = useState("");
  const [busy, setBusy] = useState(false);
  const github = settings?.github;
  // The form follows what is stored (on load and after every save).
  useEffect(() => {
    if (!github) return;
    setHost(github.host);
    setAccount(github.account ?? "");
  }, [github]);

  const dirty =
    !!github &&
    (host.trim() !== github.host || account.trim() !== (github.account ?? ""));

  const submit = async () => {
    if (!github) return;
    setBusy(true);
    const ok = await save({
      github: { ...github, host: host.trim(), account: account.trim() || null },
    });
    setBusy(false);
    if (ok) toastSuccess("GitHub settings saved");
  };

  return (
    <section className={styles.section}>
      <h3 className={styles.sectionLabel}>GitHub</h3>
      <SettingToggle
        label="Use GitHub"
        help="Through the gh CLI and its login. Run gh auth login once."
        value={github?.enabled ?? false}
        disabled={!github}
        onChange={(enabled) =>
          github && void save({ github: { ...github, enabled } })
        }
      />
      <form
        className={styles.card}
        aria-label="GitHub host and account"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <div className={styles.field}>
          <label htmlFor="gh-host">Host</label>
          <input
            id="gh-host"
            type="text"
            value={host}
            placeholder="github.com"
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            disabled={!github}
            onChange={(e) => setHost(e.target.value)}
          />
        </div>
        <div className={styles.field}>
          <label htmlFor="gh-account">Account (optional)</label>
          <input
            id="gh-account"
            type="text"
            value={account}
            placeholder="Blank = gh's active account"
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            disabled={!github}
            onChange={(e) => setAccount(e.target.value)}
          />
        </div>
        <div className={styles.editActions}>
          <button type="submit" className="primary" disabled={!dirty || busy}>
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </form>
    </section>
  );
}

function GitlabTab() {
  const settings = useBackendSettingsStore((s) => s.settings);
  const tokenPresent = useBackendSettingsStore((s) => s.tokenPresent);
  const save = useBackendSettingsStore((s) => s.save);
  const storeToken = useBackendSettingsStore((s) => s.setGitlabToken);
  const [url, setUrl] = useState("");
  // Always starts empty: the stored token is never read back.
  const [token, setToken] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const gitlab = settings?.gitlab;
  useEffect(() => {
    if (gitlab) setUrl(gitlab.baseUrl);
  }, [gitlab]);

  const urlChanged = !!gitlab && url.trim() !== gitlab.baseUrl;
  const dirty = urlChanged || token.trim() !== "";

  const submit = async () => {
    if (!gitlab) return;
    const trimmed = token.trim();
    if (trimmed && !url.trim()) {
      setError("Set the GitLab URL before its token.");
      return;
    }
    setError(null);
    setBusy(true);
    // Settings first: the token is stored for the GitLab URL just saved.
    let ok = true;
    if (urlChanged) ok = await save({ gitlab: { ...gitlab, baseUrl: url } });
    if (ok && trimmed) ok = await storeToken(trimmed);
    setBusy(false);
    if (ok) {
      setToken("");
      toastSuccess("GitLab settings saved");
    }
  };

  const remove = async () => {
    setBusy(true);
    const ok = await storeToken("");
    setBusy(false);
    if (ok) toastSuccess("Token removed");
  };

  return (
    <section className={styles.section}>
      <h3 className={styles.sectionLabel}>GitLab</h3>
      <small className={styles.rowHelp}>
        A self-hosted or gitlab.com instance, read with a personal access token
        (read_api scope is enough). Leave the URL empty to turn GitLab off.
      </small>
      <form
        className={styles.card}
        aria-label="GitLab URL and token"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <div className={styles.field}>
          <label htmlFor="gl-url">Base URL</label>
          <input
            id="gl-url"
            type="text"
            value={url}
            placeholder="https://gitlab.example.com (empty = off)"
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            disabled={!gitlab}
            onChange={(e) => {
              setUrl(e.target.value);
              setError(null);
            }}
          />
        </div>
        <div className={styles.field}>
          <div className={styles.labelRow}>
            <label htmlFor="gl-token">Personal access token</label>
            <span
              className={styles.tokenState}
              data-set={tokenPresent ? "true" : "false"}
            >
              {tokenPresent ? "Token set" : "Not set"}
            </span>
          </div>
          <div className={styles.fieldRow}>
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
              disabled={!gitlab}
              onChange={(e) => {
                setToken(e.target.value);
                setError(null);
              }}
            />
            {tokenPresent && (
              <button
                type="button"
                disabled={busy}
                onClick={() => void remove()}
              >
                Remove token
              </button>
            )}
          </div>
          <small className={styles.rowHelp}>Kept in the system keychain.</small>
        </div>
        {error && (
          <p className={styles.error} role="alert">
            {error}
          </p>
        )}
        <div className={styles.editActions}>
          <button type="submit" className="primary" disabled={!dirty || busy}>
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </form>
    </section>
  );
}

function AiTab() {
  const settings = useBackendSettingsStore((s) => s.settings);
  const save = useBackendSettingsStore((s) => s.save);
  const readOnly = useReadOnlyGuaranteed();
  const ai = settings?.ai;
  return (
    <section className={styles.section}>
      <h3 className={styles.sectionLabel}>AI</h3>
      <SettingToggle
        label="AI analysis"
        help="Analyses new reviews and rounds with your local agent (Claude Code or auggie)."
        value={ai?.enabled ?? false}
        disabled={!ai}
        onChange={(enabled) => ai && void save({ ai: { ...ai, enabled } })}
      />
      {!readOnly && <p className={styles.warning}>{NOT_READ_ONLY_NOTE}</p>}
    </section>
  );
}

/** The row being added (`original` null) or edited. */
interface CloneDraft {
  original: string | null;
  repo: string;
  path: string;
}

/** Native folder picker for a local clone. */
async function pickFolder(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({
    multiple: false,
    directory: true,
    title: "Choose the local clone",
  });
  return typeof picked === "string" ? picked : null;
}

function ClonesSection() {
  const settings = useBackendSettingsStore((s) => s.settings);
  const save = useBackendSettingsStore((s) => s.save);
  const [draft, setDraft] = useState<CloneDraft | null>(null);
  const [error, setError] = useState<string | null>(null);

  const clones = settings?.clonePaths ?? {};
  const rows = Object.entries(clones).sort(([a], [b]) => a.localeCompare(b));

  const edit = (next: CloneDraft | null) => {
    setDraft(next);
    setError(null);
  };
  const choose = async () => {
    if (!draft) return;
    try {
      const path = await pickFolder();
      if (path) setDraft({ ...draft, path });
    } catch (e) {
      setError(toIpcError(e).message);
    }
  };
  const submit = () => {
    if (!draft) return;
    const repo = normalizeRepo(draft.repo);
    const problem = cloneError(repo, draft.path, clones, draft.original);
    if (problem) {
      setError(problem);
      return;
    }
    edit(null);
    void save({
      clonePaths: withClone(clones, repo, draft.path, draft.original),
    });
  };

  return (
    <section className={styles.section}>
      <h3 className={styles.sectionLabel}>Local clones</h3>
      <small className={styles.rowHelp}>
        A local checkout per repository lets the analysis read more than the
        diff. Recommended, never required.
      </small>
      {settings && rows.length === 0 && !draft && (
        <p className={styles.empty}>No clones set.</p>
      )}
      {rows.map(([repo, path]) => (
        <div key={repo} className={styles.card}>
          <div className={styles.cardHeader}>
            <span className={styles.cardName}>{repo}</span>
            <div className={`${styles.cardActions} ${styles.pushRight}`}>
              <button
                type="button"
                aria-label={`Edit the clone of ${repo}`}
                onClick={() => edit({ original: repo, repo, path })}
              >
                Edit
              </button>
              <button
                type="button"
                aria-label={`Remove the clone of ${repo}`}
                onClick={() =>
                  void save({ clonePaths: withoutClone(clones, repo) })
                }
              >
                Remove
              </button>
            </div>
          </div>
          <span className={styles.cardDetail} title={path}>
            {path}
          </span>
        </div>
      ))}
      {draft ? (
        <form
          className={styles.card}
          aria-label={draft.original ? "Edit clone" : "New clone"}
          onSubmit={(e) => {
            e.preventDefault();
            submit();
          }}
        >
          <span className={styles.cardName}>
            {draft.original ? `Edit ${draft.original}` : "New clone"}
          </span>
          <div className={styles.field}>
            <label htmlFor="clone-repo">Repository</label>
            <input
              id="clone-repo"
              type="text"
              value={draft.repo}
              placeholder="github.com/acme/widgets"
              autoComplete="off"
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
              onChange={(e) => setDraft({ ...draft, repo: e.target.value })}
            />
          </div>
          <div className={styles.field}>
            <label htmlFor="clone-path">Folder</label>
            <div className={styles.fieldRow}>
              <input
                id="clone-path"
                type="text"
                readOnly
                value={draft.path}
                placeholder="Not chosen"
                title={draft.path}
              />
              <button type="button" onClick={() => void choose()}>
                Choose…
              </button>
            </div>
          </div>
          {error && (
            <p className={styles.error} role="alert">
              {error}
            </p>
          )}
          <div className={styles.editActions}>
            <button type="button" onClick={() => edit(null)}>
              Cancel
            </button>
            <button type="submit" className="primary">
              Save
            </button>
          </div>
        </form>
      ) : (
        <div className={styles.editActions}>
          <button
            type="button"
            disabled={!settings}
            onClick={() => edit({ original: null, repo: "", path: "" })}
          >
            Add clone
          </button>
        </div>
      )}
    </section>
  );
}
