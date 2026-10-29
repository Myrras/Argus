/**
 * Settings → Scripts: the user's own submit scripts (`settings.json`
 * `submitScripts`). On top what they are, when they run and what they get
 * (the `ARGUS_*` table, stdin, working folder, timeout, the trust and
 * `chmod +x` lines) with a collapsible example; where scripts written with
 * the AI are saved ("Show in Finder"); the "Write with AI" drafts (running
 * in the background: status, Stop, Open, Discard); then one card per
 * script, in the order they run, "Add script" and "Write with AI".
 *
 * The family's save model: a card's name, command and arguments sit in a
 * form with its own Save; verdicts, repositories, "Ticked by default", the
 * timeout, moving and removing apply at once. Saves go through
 * `backendSettingsStore` (the core tidies the scripts and returns them).
 * Each card shows its last run (`scriptResultsStore`, from `scripts:done`).
 */

import { useEffect, useMemo, useState } from "react";

import {
  cancelScriptDraft,
  discardScriptDraft,
  revealSubmitScriptsDir,
  toIpcError,
} from "../ipc/commands";
import type { ReviewVerdict, ScriptDraft, SubmitScript } from "../ipc/types";
import {
  ALL_VERDICTS,
  argsText,
  draftStatusLabel,
  progressSnippet,
  EXAMPLE_SCRIPT,
  insideDir,
  moveScript,
  newScript,
  parseArgs,
  repoKey,
  requestCount,
  SCRIPT_ENV,
  scriptName,
  STATUS_LABEL,
  TIMEOUT_MAX,
  TIMEOUT_MIN,
  VERDICT_CHOICE,
  withScript,
} from "../lib/submitScripts";
import { ago, fullTime } from "../lib/time";
import { useNow } from "../lib/useNow";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { useReviewStore } from "../state/reviewStore";
import { useScriptResultsStore } from "../state/scriptResultsStore";
import { useSubmitScriptsStore } from "../state/submitScriptsStore";
import { toastError } from "../state/toastStore";
import { CodeBlock } from "./Markdown";
import { SettingToggle, Stepper } from "./SettingsControls";
import styles from "./SettingsModal.module.css";
import own from "./ScriptsTab.module.css";

/** Native picker for a script file. */
async function pickScript(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({
    multiple: false,
    directory: false,
    title: "Choose the script",
  });
  return typeof picked === "string" ? picked : null;
}

export function ScriptsTab() {
  const settings = useBackendSettingsStore((s) => s.settings);
  const save = useBackendSettingsStore((s) => s.save);
  const refresh = useSubmitScriptsStore((s) => s.refresh);
  const generation = useSubmitScriptsStore((s) => s.generation);
  const openWriter = useSubmitScriptsStore((s) => s.openWriter);
  const scripts = settings?.submitScripts ?? [];

  // Whether "Write with AI" can run depends on the AI settings: read on
  // open and again when they change.
  const aiKey = settings ? JSON.stringify(settings.ai) : "";
  useEffect(() => {
    void refresh();
  }, [refresh, aiKey]);

  const saveScripts = (next: SubmitScript[]) => save({ submitScripts: next });

  const canWrite = !!generation?.available;
  const writeTitle = !generation
    ? "Checking the AI agent…"
    : (generation.reason ?? "Describe a script; the AI writes it for you");

  return (
    <>
      <ScriptsExplainer />
      <DraftsSection />
      <section className={styles.section}>
        <h3 className={styles.sectionLabel}>Your scripts</h3>
        {settings && scripts.length === 0 && (
          <p className={styles.empty}>No scripts yet.</p>
        )}
        {scripts.map((script, i) => (
          <ScriptCard
            key={script.id}
            script={script}
            first={i === 0}
            last={i === scripts.length - 1}
            onSave={(patch) =>
              saveScripts(withScript(scripts, script.id, patch))
            }
            onMove={(delta) =>
              void saveScripts(moveScript(scripts, script.id, delta))
            }
            onRemove={() =>
              void saveScripts(scripts.filter((s) => s.id !== script.id))
            }
            canWrite={canWrite}
            writeTitle={writeTitle}
          />
        ))}
        <div className={styles.editActions}>
          <span
            className={own.writeWrap}
            title={canWrite ? undefined : writeTitle}
          >
            <button
              type="button"
              disabled={!settings || !canWrite}
              aria-describedby={canWrite ? undefined : "write-ai-reason"}
              onClick={() => openWriter(null)}
            >
              Write with AI…
            </button>
          </span>
          <button
            type="button"
            disabled={!settings}
            onClick={() => void saveScripts([...scripts, newScript()])}
          >
            Add script
          </button>
        </div>
        {generation && !generation.available && (
          <small id="write-ai-reason" className={styles.rowHelp}>
            Write with AI is off: {generation.reason}
          </small>
        )}
      </section>
    </>
  );
}

/** What submit scripts are and what they get. */
function ScriptsExplainer() {
  const dir = useSubmitScriptsStore((s) => s.dir);
  const reveal = async () => {
    try {
      await revealSubmitScriptsDir();
    } catch (e) {
      toastError("Could not show the scripts folder", toIpcError(e).message);
    }
  };
  return (
    <section className={styles.section}>
      <h3 className={styles.sectionLabel}>Submit scripts</h3>
      <div className={own.prose}>
        <p>
          Your own scripts that run on your machine after you submit a review in
          Argus — to tick the merge request&apos;s checklist, post to chat,
          start a job. A script runs only after Submit review succeeded, and
          only when you tick it in the submit dialog. The ticked ones run one
          after the other, in the order listed here; the dialog does not wait
          for them, a toast tells you how they went.
        </p>
        <p>Each script gets these environment variables:</p>
        <table className={own.envTable}>
          <tbody>
            {SCRIPT_ENV.map(([name, what]) => (
              <tr key={name}>
                <td>
                  <code>{name}</code>
                </td>
                <td>{what}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <p>
          The same comes as one JSON object on stdin (keys <code>provider</code>
          , <code>host</code>, <code>repo</code>, <code>number</code>,{" "}
          <code>url</code>, <code>title</code>, <code>verdict</code>,{" "}
          <code>headSha</code>, <code>clone</code>), plus <code>summary</code>:
          the summary you wrote.
        </p>
        <ul className={own.facts}>
          <li>
            It runs in the repository&apos;s local clone when you set one
            (Settings → Repositories), else in an empty temporary folder.
          </li>
          <li>
            It is stopped after its timeout: 60 seconds unless you change it
            (5–600).
          </li>
          <li>
            Argus starts the file directly, without a shell: it needs a first
            line such as <code>#!/bin/sh</code> and must be executable (
            <code>chmod +x your-script.sh</code>).
          </li>
          <li>
            It runs with your own tools and logins (<code>glab</code>,{" "}
            <code>gh</code>). Argus never passes its token: token variables are
            removed from the environment.
          </li>
          <li>
            The last 16 KB of its output shows on its card below after a run.
          </li>
        </ul>
        <p className={styles.warning} role="note">
          Argus can&apos;t check what a script does. Only add scripts you trust.
        </p>
        <details className={own.example}>
          <summary>Example: tick every checklist item (GitLab)</summary>
          <CodeBlock info="sh" code={EXAMPLE_SCRIPT.trimEnd()} />
          <small className={styles.rowHelp}>
            Needs <code>glab</code> (logged in) and <code>jq</code>. Save it,
            run <code>chmod +x</code> on it, add it below for Approve.
          </small>
        </details>
        <p>
          <strong>Write with AI</strong> drafts a script for you: describe it,
          and your AI agent writes it in the background, so you can close the
          dialog and Settings and keep reviewing. Running drafts are listed
          under Drafts below, where you can stop them. Once a script is written,
          send follow-ups to change it: each one keeps your earlier requests as
          context. Drafts last until Argus quits; nothing is saved or run until
          you save the script and tick it when submitting.
        </p>
        {dir && (
          <div className={own.dirRow}>
            <span className={own.dirText}>
              Scripts written with AI are saved in{" "}
              <code className={own.path} title={dir}>
                {dir}
              </code>
            </span>
            <button type="button" onClick={() => void reveal()}>
              Show in Finder
            </button>
          </div>
        )}
      </div>
    </section>
  );
}

/** Stop or forget a draft from the Scripts tab; failures toast. */
async function stopDraft(d: ScriptDraft): Promise<void> {
  try {
    useSubmitScriptsStore.getState().applyDraft(await cancelScriptDraft(d.id));
  } catch (e) {
    toastError(`Could not stop ${d.title}`, toIpcError(e).message);
  }
}

async function discardDraft(d: ScriptDraft): Promise<void> {
  try {
    await discardScriptDraft(d.id);
    useSubmitScriptsStore.getState().removeDraft(d.id);
  } catch (e) {
    toastError(`Could not discard ${d.title}`, toIpcError(e).message);
  }
}

/** The "Write with AI" drafts of this session, when there are any. */
function DraftsSection() {
  const drafts = useSubmitScriptsStore((s) => s.drafts);
  const openDraft = useSubmitScriptsStore((s) => s.openDraft);
  if (drafts.length === 0) return null;
  return (
    <section className={styles.section} aria-label="Drafts">
      <h3 className={styles.sectionLabel}>Drafts</h3>
      <ul className={own.drafts}>
        {drafts.map((d) => {
          const running = d.status === "running";
          return (
            <li
              key={d.id}
              className={own.draft}
              aria-label={`Draft ${d.title}`}
            >
              <div className={own.draftText}>
                <span className={own.draftTitle}>{d.title}</span>
                <span className={own.draftStatus} data-status={d.status}>
                  {running && <span className="spinner" aria-hidden="true" />}
                  {draftStatusLabel(d)} · {requestCount(d)}
                </span>
                {running && progressSnippet(d.progress) && (
                  <span className={own.draftSnippet}>
                    {progressSnippet(d.progress)}
                  </span>
                )}
              </div>
              <div className={styles.cardActions}>
                {running && (
                  <button
                    type="button"
                    aria-label={`Stop ${d.title}`}
                    onClick={() => void stopDraft(d)}
                  >
                    Stop
                  </button>
                )}
                <button
                  type="button"
                  aria-label={`Open ${d.title}`}
                  onClick={() => openDraft(d.id)}
                >
                  Open
                </button>
                <button
                  type="button"
                  aria-label={`Discard ${d.title}`}
                  onClick={() => void discardDraft(d)}
                >
                  Discard
                </button>
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

/** Repositories the card offers: local clones and reviews Argus knows. */
function useRepoSuggestions(): string[] {
  const clonePaths = useBackendSettingsStore((s) => s.settings?.clonePaths);
  const reviews = useReviewStore((s) => s.reviews);
  return useMemo(() => {
    const all = new Set<string>(Object.keys(clonePaths ?? {}));
    for (const r of reviews) all.add(repoKey(r));
    return [...all].sort((a, b) => a.localeCompare(b));
  }, [clonePaths, reviews]);
}

function ScriptCard({
  script,
  first,
  last,
  onSave,
  onMove,
  onRemove,
  canWrite,
  writeTitle,
}: {
  script: SubmitScript;
  first: boolean;
  last: boolean;
  onSave: (patch: Partial<SubmitScript>) => Promise<boolean>;
  onMove: (delta: number) => void;
  onRemove: () => void;
  canWrite: boolean;
  writeTitle: string;
}) {
  const dir = useSubmitScriptsStore((s) => s.dir);
  const openWriter = useSubmitScriptsStore((s) => s.openWriter);
  const suggestions = useRepoSuggestions();
  const [name, setName] = useState(script.name);
  const [command, setCommand] = useState(script.command);
  const [args, setArgs] = useState(argsText(script.args));
  const [repo, setRepo] = useState("");
  const [busy, setBusy] = useState(false);
  // The form follows what is stored (on load and after every save).
  useEffect(() => {
    setName(script.name);
    setCommand(script.command);
    setArgs(argsText(script.args));
  }, [script.name, script.command, script.args]);

  const label = scriptName(script);
  const dirty =
    name.trim() !== script.name ||
    command.trim() !== script.command ||
    argsText(parseArgs(args)) !== argsText(script.args);
  const fieldId = (f: string) => `script-${script.id}-${f}`;

  const submit = async () => {
    setBusy(true);
    await onSave({
      name: name.trim(),
      command: command.trim(),
      args: parseArgs(args),
    });
    setBusy(false);
  };
  const choose = async () => {
    try {
      const path = await pickScript();
      if (path) setCommand(path);
    } catch (e) {
      toastError("Could not open the file picker", toIpcError(e).message);
    }
  };
  const toggleVerdict = (v: ReviewVerdict, on: boolean) => {
    const verdicts = on
      ? ALL_VERDICTS.filter((x) => x === v || script.verdicts.includes(x))
      : script.verdicts.filter((x) => x !== v);
    void onSave({ verdicts });
  };
  const addRepo = () => {
    const r = repo.trim().replace(/\/+$/, "");
    if (!r) return;
    setRepo("");
    if (script.repos.some((k) => k.toLowerCase() === r.toLowerCase())) return;
    void onSave({ repos: [...script.repos, r] });
  };
  const editable = insideDir(script.command, dir);
  const draft = useSubmitScriptsStore((s) =>
    s.drafts.find((d) => d.scriptId === script.id),
  );
  const openDraft = useSubmitScriptsStore((s) => s.openDraft);

  return (
    <div className={styles.card} aria-label={`Script ${label}`} role="group">
      <div className={styles.cardHeader}>
        <span className={styles.cardName}>{label}</span>
        <div className={`${styles.cardActions} ${styles.pushRight}`}>
          {editable && (
            <span title={canWrite ? undefined : writeTitle}>
              <button
                type="button"
                disabled={!canWrite}
                aria-label={`Edit ${label} with AI`}
                onClick={() => openWriter(script.id)}
              >
                Edit with AI…
              </button>
            </span>
          )}
          <button
            type="button"
            className="ghost"
            disabled={first}
            aria-label={`Move ${label} up`}
            title="Runs earlier"
            onClick={() => onMove(-1)}
          >
            ↑
          </button>
          <button
            type="button"
            className="ghost"
            disabled={last}
            aria-label={`Move ${label} down`}
            title="Runs later"
            onClick={() => onMove(1)}
          >
            ↓
          </button>
          <button
            type="button"
            aria-label={`Remove ${label}`}
            onClick={onRemove}
          >
            Remove
          </button>
        </div>
      </div>

      {draft && (
        <div className={own.linked}>
          <span>
            Draft with changes
            {draft.status === "running" ? " (writing…)" : ""}
          </span>
          <button
            type="button"
            aria-label={`Open the draft of ${label}`}
            onClick={() => openDraft(draft.id)}
          >
            Open
          </button>
        </div>
      )}

      <form
        className={own.form}
        aria-label={`${label}: name, command and arguments`}
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <div className={styles.field}>
          <label htmlFor={fieldId("name")}>Name</label>
          <input
            id={fieldId("name")}
            type="text"
            value={name}
            autoComplete="off"
            spellCheck={false}
            onChange={(e) => setName(e.target.value)}
          />
        </div>
        <div className={styles.field}>
          <label htmlFor={fieldId("command")}>Command</label>
          <div className={styles.fieldRow}>
            <input
              id={fieldId("command")}
              type="text"
              value={command}
              placeholder="/Users/you/bin/tick-checklist.sh"
              autoComplete="off"
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
              onChange={(e) => setCommand(e.target.value)}
            />
            <button type="button" onClick={() => void choose()}>
              Choose…
            </button>
          </div>
          <small className={styles.rowHelp}>
            A full path, <code>~/…</code>, or a program on your PATH.
          </small>
        </div>
        <div className={styles.field}>
          <label htmlFor={fieldId("args")}>Arguments (optional)</label>
          <textarea
            id={fieldId("args")}
            className={own.args}
            rows={2}
            value={args}
            spellCheck={false}
            placeholder="One per line"
            onChange={(e) => setArgs(e.target.value)}
          />
          <small className={styles.rowHelp}>
            One per line, passed as they are (no shell: no quotes or variables
            needed).
          </small>
        </div>
        <div className={styles.editActions}>
          <button type="submit" className="primary" disabled={!dirty || busy}>
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </form>

      <fieldset className={own.verdicts}>
        <legend className={own.legend}>Offer it when you</legend>
        {ALL_VERDICTS.map((v) => (
          <label key={v} className={own.check}>
            <input
              type="checkbox"
              checked={script.verdicts.includes(v)}
              onChange={(e) => toggleVerdict(v, e.target.checked)}
            />
            {VERDICT_CHOICE[v]}
          </label>
        ))}
      </fieldset>
      {script.verdicts.includes("comment") && (
        <small className={styles.rowHelp}>
          Comment reviews are GitHub only.
        </small>
      )}
      {script.verdicts.length === 0 && (
        <p className={styles.warning} role="note">
          No verdict ticked: this script is never offered.
        </p>
      )}

      <div className={styles.field}>
        <label htmlFor={fieldId("repo")}>Repositories</label>
        {script.repos.length === 0 ? (
          <small className={styles.rowHelp}>Every repository.</small>
        ) : (
          <ul className={own.chips} aria-label={`Repositories of ${label}`}>
            {script.repos.map((r) => (
              <li key={r} className={own.chip}>
                <span className="mono">{r}</span>
                <button
                  type="button"
                  className="ghost"
                  aria-label={`Remove ${r} from ${label}`}
                  onClick={() =>
                    void onSave({
                      repos: script.repos.filter((k) => k !== r),
                    })
                  }
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
        )}
        <div className={styles.fieldRow}>
          <input
            id={fieldId("repo")}
            type="text"
            list={fieldId("repos")}
            value={repo}
            placeholder="gitlab.example.com/group/project"
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            onChange={(e) => setRepo(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                addRepo();
              }
            }}
          />
          <datalist id={fieldId("repos")}>
            {suggestions.map((s) => (
              <option key={s} value={s} />
            ))}
          </datalist>
          <button
            type="button"
            disabled={!repo.trim()}
            aria-label={`Add the repository to ${label}`}
            onClick={addRepo}
          >
            Add
          </button>
        </div>
        <small className={styles.rowHelp}>
          Limit it to these repositories (host/path). None: every repository.
        </small>
      </div>

      <SettingToggle
        label="Ticked by default"
        help="Pre-ticked in the submit dialog; you can still untick it there."
        value={script.defaultOn}
        onChange={(defaultOn) => void onSave({ defaultOn })}
      />
      <div className={styles.row}>
        <span className={styles.rowLabel}>
          Timeout
          <small className={styles.rowHelp}>Stopped after this long.</small>
        </span>
        <Stepper
          label={`Timeout of ${label}`}
          value={script.timeoutSecs}
          min={TIMEOUT_MIN}
          max={TIMEOUT_MAX}
          step={5}
          show={(n) => `${n} s`}
          onChange={(timeoutSecs) => void onSave({ timeoutSecs })}
        />
      </div>
      <LastRun scriptId={script.id} />
    </div>
  );
}

/** The script's last run: status, when, and its output on request. */
function LastRun({ scriptId }: { scriptId: string }) {
  const run = useScriptResultsStore((s) => s.last[scriptId]);
  const now = useNow();
  if (!run) {
    return <small className={styles.rowHelp}>Not run yet.</small>;
  }
  const r = run.result;
  const output = [
    r.message ? `${r.message}\n` : "",
    r.stdout ? `stdout:\n${r.stdout}` : "",
    r.stderr ? `stderr:\n${r.stderr}` : "",
  ]
    .filter(Boolean)
    .join("\n");
  return (
    <details className={own.lastRun} data-status={r.status}>
      <summary>
        Last run:{" "}
        <span className={own.status} data-status={r.status}>
          {STATUS_LABEL[r.status]}
          {r.exitCode !== null && r.status !== "ok" ? ` (${r.exitCode})` : ""}
        </span>{" "}
        <span title={fullTime(r.finishedAt)}>{ago(r.finishedAt, now)}</span>
        {r.durationMs > 0 && ` · ${(r.durationMs / 1000).toFixed(1)} s`}
      </summary>
      {output ? (
        <pre className={own.output}>{output}</pre>
      ) : (
        <small className={styles.rowHelp}>No output.</small>
      )}
    </details>
  );
}
