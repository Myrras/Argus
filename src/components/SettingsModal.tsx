/**
 * App settings dialog, tabbed like Iris's and Podium's.
 *
 * - General: theme and reduced motion (`themeStore` / `settingsStore`), the
 *   refresh interval and the disk cache's size limit (the core's
 *   `settings.json`), with what the cache holds and Clear (`cacheStore`).
 * - Accounts: every host the user's `gh` and `glab` are logged in to, each
 *   with its login state, an on/off switch and (gh) the account to use;
 *   the login commands, copyable, and Check again (`AccountsTab`). Argus
 *   keeps no token.
 * - AI: analysis on/off, the default review language (`ReviewLanguageField`)
 *   and a pointer to Repositories.
 * - Repositories: per repository its local clone, review skill and review
 *   language (`RepositoriesTab`), opened on one by the clone chip.
 * - Editors: which application opens a review's file from its local clone
 *   (Open in editor): a default editor and per-extension overrides, Iris's
 *   Editors tab (`editorOverrides`, `defaultEditor` in `settings.json`).
 * - Scripts: the user's own submit scripts (`ScriptsTab`): what they are and
 *   get, the "Write with AI" drafts, one card per script, and the writer
 *   (`ScriptWriter`, a dialog over this one: Escape closes it first; the
 *   draft keeps running). The tab shows a spinner while a draft runs.
 * - Shortcuts: rebind the keyboard shortcuts (`ShortcutsTab`, `lib/keymap`).
 *
 * The family's save model: switches and steppers apply at once; text fields
 * sit in a form with its own Save. Backend failures are error toasts, input
 * the dialog can check itself is refused inline.
 *
 * A new tab: add the id to `SettingsTabId`, an entry to `TABS` and a branch
 * in `TabBody`.
 */

import { useEffect, useRef, useState } from "react";

import { toIpcError } from "../ipc/commands";
import {
  appName,
  normalizeExtension,
  overrideError,
  withOverride,
  withoutOverride,
} from "../lib/editorOverrides";
import { isMac, isWindows } from "../lib/platform";
import { AUTO_LANGUAGE, globalLanguage } from "../lib/repositories";
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
import {
  CACHE_LIMITS_MB,
  DEFAULT_CACHE_LIMIT_MB,
  formatBytes,
  formatLimit,
  useCacheStore,
} from "../state/cacheStore";
import { useSettingsStore } from "../state/settingsStore";
import { useSubmitScriptsStore } from "../state/submitScriptsStore";
import { useThemeStore, type ThemeMode } from "../state/themeStore";
import { AccountsTab } from "./AccountsTab";
import { CheckIcon } from "./icons";
import { Modal } from "./Modal";
import { RepositoriesTab } from "./RepositoriesTab";
import { ReviewLanguageField } from "./ReviewLanguageField";
import { ScriptsTab } from "./ScriptsTab";
import { ScriptWriterHost } from "./ScriptWriter";
import { SettingToggle, Stepper } from "./SettingsControls";
import { ShortcutsTab } from "./ShortcutsTab";
import styles from "./SettingsModal.module.css";

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

const TABS: [SettingsTabId, string][] = [
  ["general", "General"],
  ["accounts", "Accounts"],
  ["ai", "AI"],
  ["repositories", "Repositories"],
  ["editors", "Editors"],
  ["scripts", "Scripts"],
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

export function SettingsModal({ open, onClose }: SettingsModalProps) {
  const [tab, setTab] = useState<SettingsTabId>("general");
  const requestedTab = useSettingsRequestStore((s) => s.request);
  // Every request counts, also one for the tab asked for before.
  const requestSeq = useSettingsRequestStore((s) => s.seq);
  useEffect(() => {
    if (open && requestedTab) setTab(requestedTab);
  }, [open, requestedTab, requestSeq]);
  // Fresh from the core every time the dialog opens.
  useEffect(() => {
    if (open) void useBackendSettingsStore.getState().load();
  }, [open]);
  // "Write with AI" sits over this dialog: Escape (or a click outside)
  // closes it first, never both.
  const writerOpen = useSubmitScriptsStore((s) => s.writer !== null);
  // A small spinner on the Scripts tab while a draft is being written.
  const draftRunning = useSubmitScriptsStore((s) =>
    s.drafts.some((d) => d.status === "running"),
  );
  useEffect(() => {
    if (!open) useSubmitScriptsStore.getState().closeWriter();
  }, [open]);
  const close = () => {
    if (!writerOpen) onClose();
  };
  // Only this pane scrolls; a new tab starts at the top, not where the last
  // one was left.
  const paneRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (paneRef.current) paneRef.current.scrollTop = 0;
  }, [tab]);

  return (
    <>
      <Modal
        open={open}
        title="Settings"
        onClose={close}
        width="min(760px, calc(100vw - 32px))"
        height="min(680px, calc(100vh - 64px))"
        bare
      >
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
              {id === "scripts" && draftRunning && (
                <span
                  className={`spinner ${styles.tabBusy}`}
                  aria-hidden="true"
                  title="A script draft is being written"
                />
              )}
            </button>
          ))}
        </div>

        <div
          ref={paneRef}
          className={styles.pane}
          role="tabpanel"
          aria-label={TABS.find(([id]) => id === tab)?.[1]}
          data-testid="settings-pane"
        >
          <TabBody tab={tab} onTab={setTab} />
        </div>
      </Modal>
      {open && <ScriptWriterHost />}
    </>
  );
}

function TabBody({
  tab,
  onTab,
}: {
  tab: SettingsTabId;
  onTab: (tab: SettingsTabId) => void;
}) {
  switch (tab) {
    case "general":
      return <GeneralTab />;
    case "accounts":
      return <AccountsTab />;
    case "ai":
      return <AiTab onRepositories={() => onTab("repositories")} />;
    case "repositories":
      return <RepositoriesTab />;
    case "editors":
      return <EditorsTab />;
    case "scripts":
      return <ScriptsTab />;
    case "shortcuts":
      return <ShortcutsTab />;
  }
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

      <CacheSection />
    </>
  );
}

/**
 * The disk cache: file contents and file lists read from GitHub and GitLab,
 * so reopening a review needs no network. Its limit is a stepper over
 * `CACHE_LIMITS_MB` (by index: the steps are not even).
 */
function CacheSection() {
  const settings = useBackendSettingsStore((s) => s.settings);
  const save = useBackendSettingsStore((s) => s.save);
  const stats = useCacheStore((s) => s.stats);
  const clearing = useCacheStore((s) => s.clearing);
  const load = useCacheStore((s) => s.load);
  const clear = useCacheStore((s) => s.clear);
  useEffect(() => {
    void load();
  }, [load]);
  const limit = settings?.cacheLimitMb ?? DEFAULT_CACHE_LIMIT_MB;
  // The nearest offered step (a hand-edited value may be between two).
  const index = settings
    ? CACHE_LIMITS_MB.reduce(
        (best, mb, i) =>
          Math.abs(mb - limit) < Math.abs(CACHE_LIMITS_MB[best] - limit)
            ? i
            : best,
        0,
      )
    : null;
  return (
    <section className={styles.section}>
      <h3 className={styles.sectionLabel}>Cache</h3>
      <div className={styles.row}>
        <span className={styles.rowLabel}>
          Keep up to
          <small className={styles.rowHelp}>
            Files and file lists read from GitHub and GitLab are kept on disk,
            so a review opens again without the network. The least recently used
            go first.
          </small>
        </span>
        <Stepper
          label="Cache limit"
          value={index}
          min={0}
          max={CACHE_LIMITS_MB.length - 1}
          show={(i) => formatLimit(CACHE_LIMITS_MB[i])}
          onChange={(i) => void save({ cacheLimitMb: CACHE_LIMITS_MB[i] })}
        />
      </div>
      <div className={styles.row}>
        <span className={styles.rowLabel}>
          In use
          <small className={styles.rowHelp} aria-live="polite">
            {stats
              ? `${formatBytes(stats.bytes)} in ${stats.entries} ${stats.entries === 1 ? "entry" : "entries"}`
              : "…"}
          </small>
        </span>
        <button
          type="button"
          disabled={clearing || !stats || stats.entries === 0}
          onClick={() => void clear()}
        >
          Clear cache
        </button>
      </div>
    </section>
  );
}

function AiTab({ onRepositories }: { onRepositories: () => void }) {
  const settings = useBackendSettingsStore((s) => s.settings);
  const save = useBackendSettingsStore((s) => s.save);
  const readOnly = useReadOnlyGuaranteed();
  const ai = settings?.ai;
  return (
    <>
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
      <section className={styles.section}>
        <h3 className={styles.sectionLabel}>Review language</h3>
        <small className={styles.rowHelp}>
          The language the analysis writes its findings, checklist and verdicts
          in, for every repository without its own. Auto follows the merge
          request (English when unclear). Code stays as it is.
        </small>
        <ReviewLanguageField
          id="ai-review-language"
          label="Default review language"
          value={globalLanguage(settings)}
          disabled={!ai}
          onChange={(language) =>
            ai &&
            void save({
              ai: { ...ai, reviewLanguage: language ?? AUTO_LANGUAGE },
            })
          }
        />
        <small className={styles.rowHelp}>
          Per-repository setup (local clone, review skill, language):{" "}
          <button
            type="button"
            className={`ghost ${styles.inlineLink}`}
            onClick={onRepositories}
          >
            Settings → Repositories
          </button>
        </small>
      </section>
    </>
  );
}

// -- Editors (Open in editor), ported from Iris's Editors tab ----------------

/** The row being added (`original` null) or edited. */
interface EditorDraft {
  original: string | null;
  ext: string;
  app: string;
}

/** Native picker for an application: `.app` bundles on macOS, `.exe` on Windows. */
async function pickApplication(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({
    multiple: false,
    directory: false,
    title: "Choose an application",
    defaultPath: isMac ? "/Applications" : undefined,
    filters: isMac
      ? [{ name: "Applications", extensions: ["app"] }]
      : isWindows
        ? [{ name: "Programs", extensions: ["exe"] }]
        : undefined,
  });
  return typeof picked === "string" ? picked : null;
}

function EditorsTab() {
  const settings = useBackendSettingsStore((s) => s.settings);
  const save = useBackendSettingsStore((s) => s.save);
  const [draft, setDraft] = useState<EditorDraft | null>(null);
  const [error, setError] = useState<string | null>(null);

  const overrides = settings?.editorOverrides ?? {};
  const rows = Object.entries(overrides).sort(([a], [b]) => a.localeCompare(b));
  const defaultEditor = settings?.defaultEditor ?? null;

  const edit = (next: EditorDraft | null) => {
    setDraft(next);
    setError(null);
  };
  const choose = async () => {
    if (!draft) return;
    try {
      const app = await pickApplication();
      if (app) setDraft({ ...draft, app });
    } catch (e) {
      setError(toIpcError(e).message);
    }
  };
  const chooseDefault = async () => {
    try {
      const app = await pickApplication();
      if (app) void save({ defaultEditor: app });
    } catch (e) {
      setError(toIpcError(e).message);
    }
  };
  const submit = () => {
    if (!draft) return;
    const ext = normalizeExtension(draft.ext);
    const problem = overrideError(ext, draft.app, overrides, draft.original);
    if (problem) {
      setError(problem);
      return;
    }
    edit(null);
    void save({
      editorOverrides: withOverride(overrides, ext, draft.app, draft.original),
    });
  };

  return (
    <section className={styles.section}>
      <h3 className={styles.sectionLabel}>Editors</h3>
      <small className={styles.rowHelp}>
        Open in editor opens a review&apos;s file from your local clone
        (Settings → Repositories) in the application set for its extension, else
        the default editor, else your system&apos;s default app.
      </small>
      <SettingToggle
        label="Open the repository with the file"
        help="With an editor set, open the clone's folder too, so the editor shows the project. The system default app opens the file only."
        value={settings?.openWithRepository ?? true}
        disabled={!settings}
        onChange={(openWithRepository) => void save({ openWithRepository })}
      />
      <div className={styles.card}>
        <div className={styles.cardHeader}>
          <span className={styles.cardName}>Default editor</span>
          <span
            className={styles.cardDetail}
            title={defaultEditor ?? undefined}
          >
            {defaultEditor ? appName(defaultEditor) : "System default"}
          </span>
          <div className={`${styles.cardActions} ${styles.pushRight}`}>
            <button
              type="button"
              disabled={!settings}
              aria-label="Choose the default editor"
              onClick={() => void chooseDefault()}
            >
              Choose…
            </button>
            {defaultEditor && (
              <button
                type="button"
                aria-label="Use the system default"
                onClick={() => void save({ defaultEditor: null })}
              >
                Reset
              </button>
            )}
          </div>
        </div>
      </div>
      {settings && rows.length === 0 && !draft && (
        <p className={styles.empty}>No editors per extension set.</p>
      )}
      {rows.map(([ext, app]) => (
        <div key={ext} className={styles.card}>
          <div className={styles.cardHeader}>
            <span className={styles.cardName}>.{ext}</span>
            <span className={styles.cardDetail} title={app}>
              {appName(app)}
            </span>
            <div className={`${styles.cardActions} ${styles.pushRight}`}>
              <button
                type="button"
                aria-label={`Edit the .${ext} editor`}
                onClick={() => edit({ original: ext, ext, app })}
              >
                Edit
              </button>
              <button
                type="button"
                aria-label={`Remove the .${ext} editor`}
                onClick={() =>
                  void save({
                    editorOverrides: withoutOverride(overrides, ext),
                  })
                }
              >
                Remove
              </button>
            </div>
          </div>
        </div>
      ))}
      {draft ? (
        <form
          className={styles.card}
          aria-label={draft.original ? "Edit editor" : "New editor"}
          onSubmit={(e) => {
            e.preventDefault();
            submit();
          }}
        >
          <span className={styles.cardName}>
            {draft.original ? `Edit .${draft.original}` : "New editor"}
          </span>
          <div className={styles.field}>
            <label htmlFor="editor-ext">Extension</label>
            <input
              id="editor-ext"
              type="text"
              value={draft.ext}
              placeholder="md"
              autoComplete="off"
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
              onChange={(e) => setDraft({ ...draft, ext: e.target.value })}
            />
          </div>
          <div className={styles.field}>
            <label htmlFor="editor-app">Application</label>
            <div className={styles.fieldRow}>
              <input
                id="editor-app"
                type="text"
                readOnly
                value={draft.app}
                placeholder="Not chosen"
                title={draft.app}
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
        <>
          {error && (
            <p className={styles.error} role="alert">
              {error}
            </p>
          )}
          <div className={styles.editActions}>
            <button
              type="button"
              disabled={!settings}
              onClick={() => edit({ original: null, ext: "", app: "" })}
            >
              Add editor
            </button>
          </div>
        </>
      )}
    </section>
  );
}
