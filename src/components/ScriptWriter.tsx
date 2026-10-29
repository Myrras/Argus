/**
 * "Write with AI" as a conversation: the user describes a submit script,
 * the AI pass' agent writes it in the background (`start_script_draft`: no
 * tools, nothing saved or run), and every follow-up ("Ask for changes…",
 * `send_script_draft`) is answered with a whole updated script, the earlier
 * requests kept as context by the core. The dialog shows the turns in
 * order: each request, then the agent's explanation and its script
 * (read-only, with copy); older versions folded, the latest open. "Edit
 * with AI" on a card starts from the saved script (`read_submit_script`
 * here for display; the core reads it again as the conversation's start).
 *
 * The draft lives in the core (`submitScriptsStore.drafts`, updated from
 * `script-draft:state`): closing this dialog, or Settings, never stops it.
 * Stop (`cancel_script_draft`) and Discard (`discard_script_draft`) are the
 * user's. Only "Save script" writes: the file in Argus' scripts folder
 * (`save_submit_script`; an existing file is replaced only after "Replace
 * it?"), then the entry in Settings → Scripts, then the draft is linked to
 * it. Nothing runs a script here: the user still ticks it when submitting.
 */

import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";

import {
  cancelScriptDraft,
  discardScriptDraft,
  linkScriptDraft,
  readSubmitScript,
  retryScriptDraft,
  saveSubmitScript,
  sendScriptDraft,
  startScriptDraft,
  toIpcError,
} from "../ipc/commands";
import type {
  DraftTurn,
  GeneratedScript,
  ReviewVerdict,
  ScriptDraft,
  ScriptDraftProgress,
  SubmitScript,
} from "../ipc/types";
import { matches } from "../lib/keymap";
import {
  ALL_VERDICTS,
  attachedLabel,
  draftStatusLabel,
  formatElapsed,
  latestScript,
  newScript,
  slugFileName,
  VERDICT_CHOICE,
  withScript,
} from "../lib/submitScripts";
import { currentKeymap } from "../lib/useKeymap";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import {
  useSubmitScriptsStore,
  type Thought,
  type WriterRequest,
} from "../state/submitScriptsStore";
import { toastSuccess } from "../state/toastStore";
import { CodeBlock } from "./Markdown";
import { Modal } from "./Modal";
import settingsStyles from "./SettingsModal.module.css";
import styles from "./ScriptWriter.module.css";

export const READ_FIRST_NOTE =
  "Read the script before you save it. Argus never runs it until you tick it when submitting.";

export const BACKGROUND_NOTE =
  "Keeps running in the background — you can close this.";

/** A turn's error when the user stopped it (the core's `STOPPED`). */
const STOPPED = "Stopped.";

/** The open "Write with AI" dialog, if any (rendered by `SettingsModal`). */
export function ScriptWriterHost() {
  const writer = useSubmitScriptsStore((s) => s.writer);
  if (!writer) return null;
  return <ScriptWriter key={writer.session} request={writer} />;
}

function ScriptWriter({ request }: { request: WriterRequest }) {
  const close = useSubmitScriptsStore((s) => s.closeWriter);
  const dir = useSubmitScriptsStore((s) => s.dir);
  const draft = useSubmitScriptsStore((s) =>
    request.draftId
      ? s.drafts.find((d) => d.id === request.draftId)
      : undefined,
  );
  const scriptId = draft?.scriptId ?? request.scriptId;
  const existing = useBackendSettingsStore((s) =>
    scriptId
      ? s.settings?.submitScripts?.find((x) => x.id === scriptId)
      : undefined,
  );
  const [message, setMessage] = useState("");
  const [name, setName] = useState("");
  const [verdicts, setVerdicts] = useState<ReviewVerdict[]>([]);
  const [sending, setSending] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [conflict, setConflict] = useState<string | null>(null);
  // Edit with AI before the first request: the saved script, to show.
  const [seed, setSeed] = useState<GeneratedScript | null>(null);
  const [loading, setLoading] = useState(!request.draftId && !!scriptId);
  const field = useRef<HTMLTextAreaElement>(null);

  const start = useRef(existing);
  useEffect(() => {
    if (request.draftId || !request.scriptId) return;
    let live = true;
    readSubmitScript(request.scriptId)
      .then((script) => {
        if (!live) return;
        const s = start.current;
        setSeed({
          name: s?.name ?? "",
          verdicts: s?.verdicts ?? ["approve"],
          script,
          explanation: "",
        });
      })
      .catch((e: unknown) => {
        if (live) setError(toIpcError(e).message);
      })
      .finally(() => {
        if (live) setLoading(false);
      });
    return () => {
      live = false;
    };
  }, [request.draftId, request.scriptId]);

  // Name and verdicts follow the newest script (as it arrives).
  const latest = draft ? latestScript(draft) : seed;
  const latestKey = draft
    ? `${draft.id}:${draft.turns.filter((t) => t.result).length}`
    : seed
      ? "seed"
      : "";
  const latestRef = useRef(latest);
  latestRef.current = latest;
  useEffect(() => {
    const l = latestRef.current;
    if (!l) return;
    setName(l.name);
    setVerdicts(l.verdicts);
    setConflict(null);
  }, [latestKey]);

  const running = draft?.status === "running";
  const hasResult = !!draft?.turns.some((t) => t.result);
  const busy = sending || saving;
  const canSend = !!message.trim() && !running && !busy && !loading;

  const send = async () => {
    if (!canSend) return;
    setSending(true);
    setError(null);
    try {
      const store = useSubmitScriptsStore.getState();
      if (draft) {
        store.applyDraft(await sendScriptDraft(draft.id, message));
      } else {
        const d = await startScriptDraft(message, request.scriptId);
        store.applyDraft(d);
        store.attachDraft(d.id);
      }
      setMessage("");
    } catch (e) {
      setError(toIpcError(e).message);
    } finally {
      setSending(false);
    }
  };

  const stop = async () => {
    if (!draft) return;
    try {
      useSubmitScriptsStore
        .getState()
        .applyDraft(await cancelScriptDraft(draft.id));
    } catch (e) {
      setError(toIpcError(e).message);
    }
  };

  const retry = async () => {
    if (!draft || busy) return;
    setSending(true);
    setError(null);
    try {
      useSubmitScriptsStore
        .getState()
        .applyDraft(await retryScriptDraft(draft.id));
    } catch (e) {
      setError(toIpcError(e).message);
    } finally {
      setSending(false);
    }
  };

  const discard = async () => {
    if (!draft) {
      close();
      return;
    }
    try {
      await discardScriptDraft(draft.id);
      useSubmitScriptsStore.getState().removeDraft(draft.id);
    } catch (e) {
      setError(toIpcError(e).message);
    }
  };

  const save = async (overwrite: boolean) => {
    if (!draft || !latest || !hasResult || !name.trim() || busy || running)
      return;
    setSaving(true);
    setError(null);
    // Changing a saved script under the same name replaces its own file.
    const own =
      !!existing &&
      !!dir &&
      `${dir.replace(/[\\/]+$/, "")}/${slugFileName(name)}` ===
        existing.command;
    try {
      const path = await saveSubmitScript(
        name.trim(),
        latest.script,
        overwrite || own,
      );
      setConflict(null);
      const settings = useBackendSettingsStore.getState().settings;
      const scripts = settings?.submitScripts ?? [];
      const patch: Partial<SubmitScript> = {
        name: name.trim(),
        command: path,
        verdicts,
      };
      const added = existing ? null : newScript(undefined, patch);
      const next = existing
        ? withScript(scripts, existing.id, patch)
        : [...scripts, added!];
      const ok = await useBackendSettingsStore
        .getState()
        .save({ submitScripts: next });
      if (ok) {
        // A later save of this draft replaces this script.
        const linkTo = existing?.id ?? added?.id;
        if (linkTo && draft.scriptId !== linkTo) {
          await linkScriptDraft(draft.id, linkTo)
            .then((d) => useSubmitScriptsStore.getState().applyDraft(d))
            .catch(() => undefined);
        }
        toastSuccess(`Saved ${name.trim()}`);
        close();
      }
    } catch (e) {
      const err = toIpcError(e);
      if (err.kind === "conflict") setConflict(err.message);
      else setError(err.message);
    } finally {
      setSaving(false);
    }
  };

  const toggle = (v: ReviewVerdict, on: boolean) =>
    setVerdicts(
      on
        ? ALL_VERDICTS.filter((x) => x === v || verdicts.includes(x))
        : verdicts.filter((x) => x !== v),
    );

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (currentKeymap().submitComment.some((b) => matches(e, b))) {
      e.preventDefault();
      e.stopPropagation();
      void send();
    }
  };

  const title = existing
    ? `Edit ${existing.name} with AI`
    : draft
      ? draft.title
      : "Write a script with AI";
  const hasTurns = !!draft?.turns.length;

  return (
    <Modal
      open
      title={title}
      onClose={close}
      width="min(680px, 94vw)"
      initialFocus={field}
      footer={
        <>
          {draft && (
            <button
              type="button"
              className={`danger ${styles.discard}`}
              disabled={saving}
              onClick={() => void discard()}
            >
              Discard
            </button>
          )}
          <button type="button" disabled={saving} onClick={close}>
            Close
          </button>
          {conflict ? (
            <button
              type="button"
              className="danger"
              disabled={busy || running}
              onClick={() => void save(true)}
            >
              Replace it
            </button>
          ) : (
            <button
              type="button"
              className="primary"
              disabled={!hasResult || !name.trim() || busy || running}
              onClick={() => void save(false)}
            >
              {saving ? "Saving…" : "Save script"}
            </button>
          )}
        </>
      }
    >
      <div className={styles.body}>
        <Conversation
          draft={draft ?? null}
          seed={seed}
          onStop={() => void stop()}
          onRetry={() => void retry()}
        />

        <div className={settingsStyles.field}>
          <label htmlFor="writer-message">
            {hasTurns
              ? "Ask for changes…"
              : "Describe what the script should do"}
          </label>
          <textarea
            id="writer-message"
            ref={field}
            className={styles.description}
            rows={3}
            value={message}
            disabled={loading}
            placeholder={
              hasTurns
                ? "Also run it when I request changes"
                : "Tick every checklist item in the MR description when I approve"
            }
            onChange={(e) => setMessage(e.target.value)}
            onKeyDown={onKeyDown}
          />
          <div className={styles.actions}>
            <button
              type="button"
              className="primary"
              disabled={!canSend}
              onClick={() => void send()}
            >
              {hasTurns ? "Send" : "Write it"}
            </button>
          </div>
          <small className={settingsStyles.rowHelp}>
            {BACKGROUND_NOTE} Your AI agent (Settings → AI) writes it without
            tools or access to your code, following the contract of Settings →
            Scripts; each request keeps the earlier ones as context.
          </small>
        </div>

        {error && (
          <p className={settingsStyles.error} role="alert">
            {error}
          </p>
        )}

        {hasResult && (
          <>
            <p className={settingsStyles.warning} role="note">
              {READ_FIRST_NOTE}
            </p>
            <div className={settingsStyles.field}>
              <label htmlFor="writer-name">Name</label>
              <input
                id="writer-name"
                type="text"
                value={name}
                autoComplete="off"
                spellCheck={false}
                disabled={busy}
                onChange={(e) => {
                  setName(e.target.value);
                  setConflict(null);
                }}
              />
              {name.trim() && (
                <small className={settingsStyles.rowHelp}>
                  Saved as <code>{slugFileName(name) || "…"}</code>
                  {dir ? ` in ${dir}` : ""}.
                </small>
              )}
            </div>
            <fieldset className={styles.verdicts}>
              <legend className={styles.legend}>Offer it when you</legend>
              {ALL_VERDICTS.map((v) => (
                <label key={v} className={styles.check}>
                  <input
                    type="checkbox"
                    checked={verdicts.includes(v)}
                    disabled={busy}
                    onChange={(e) => toggle(v, e.target.checked)}
                  />
                  {VERDICT_CHOICE[v]}
                </label>
              ))}
            </fieldset>
            {conflict && (
              <p className={settingsStyles.warning} role="alert">
                {conflict} Replace it?
              </p>
            )}
          </>
        )}
      </div>
    </Modal>
  );
}

/** The turns in order: each request, then what came back. */
function Conversation({
  draft,
  seed,
  onStop,
  onRetry,
}: {
  draft: ScriptDraft | null;
  seed: GeneratedScript | null;
  onStop: () => void;
  onRetry: () => void;
}) {
  const thought = useSubmitScriptsStore((s) =>
    draft ? s.thoughts[draft.id] : undefined,
  );
  const base = draft?.base ?? seed;
  const turns = draft?.turns ?? [];
  const lastResult = turns.reduce((at, t, i) => (t.result ? i : at), -1);
  if (!base && turns.length === 0) return null;
  let version = base ? 1 : 0;
  const last = turns.length - 1;
  const ended = draft?.status === "failed" || draft?.status === "cancelled";
  return (
    <ol className={styles.turns} aria-label="Conversation">
      {base && (
        <li className={styles.turn}>
          <details className={styles.version} open={lastResult < 0}>
            <summary>The saved script you start from</summary>
            <CodeBlock info="sh" code={base.script.trimEnd()} />
          </details>
        </li>
      )}
      {turns.map((t, i) => {
        if (t.result) version += 1;
        const running = !!draft && i === last && draft.status === "running";
        return (
          <Turn
            key={`${i}:${t.at}`}
            turn={t}
            version={version}
            latest={i === lastResult}
            running={
              running
                ? {
                    label: draftStatusLabel(draft),
                    progress: draft.waiting ? null : draft.progress,
                  }
                : null
            }
            thought={!running && thought?.turn === i ? thought : null}
            onStop={onStop}
            onRetry={ended && i === last && !t.result ? onRetry : null}
          />
        );
      })}
    </ol>
  );
}

function Turn({
  turn,
  version,
  latest,
  running,
  thought,
  onStop,
  onRetry,
}: {
  turn: DraftTurn;
  version: number;
  latest: boolean;
  running: { label: string; progress: ScriptDraftProgress | null } | null;
  thought: Thought | null;
  onStop: () => void;
  onRetry: (() => void) | null;
}) {
  const r = turn.result;
  const attached = attachedLabel(turn.attachments ?? []);
  const notes = turn.notes ?? [];
  return (
    <li className={styles.turn}>
      <p className={styles.prompt} aria-label="Your request">
        {turn.prompt}
      </p>
      {(attached || notes.length > 0) && (
        <div className={styles.attached}>
          {attached && <span>{attached}</span>}
          {notes.map((n) => (
            <span key={n} className={styles.note}>
              {n}
            </span>
          ))}
        </div>
      )}
      {running ? (
        <RunningTurn {...running} onStop={onStop} />
      ) : (
        thought && (
          <Thinking
            text={thought.thinking}
            cut={thought.thinkingCut}
            live={false}
            summary={`Thought for ${formatElapsed(thought.ms)}`}
          />
        )
      )}
      {r ? (
        latest ? (
          <div className={styles.answer}>
            {r.explanation && (
              <p className={styles.explanation}>{r.explanation}</p>
            )}
            <CodeBlock info="sh" code={r.script.trimEnd()} />
          </div>
        ) : (
          <details className={styles.version}>
            <summary>
              Version {version}: {r.name}
            </summary>
            {r.explanation && (
              <p className={styles.explanation}>{r.explanation}</p>
            )}
            <CodeBlock info="sh" code={r.script.trimEnd()} />
          </details>
        )
      ) : running ? null : turn.error ? (
        <div className={styles.failed}>
          {turn.error === STOPPED ? (
            <p className={styles.working}>Stopped.</p>
          ) : (
            <p className={settingsStyles.error}>{turn.error}</p>
          )}
          {onRetry && (
            <button type="button" onClick={onRetry}>
              Retry
            </button>
          )}
        </div>
      ) : null}
    </li>
  );
}

/** Now, every second while `on`. */
function useNow(on: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!on) return;
    setNow(Date.now());
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [on]);
  return now;
}

/** The running turn: stage, elapsed time, Stop, then the live thinking. */
function RunningTurn({
  label,
  progress,
  onStop,
}: {
  label: string;
  progress: ScriptDraftProgress | null;
  onStop: () => void;
}) {
  const now = useNow(!!progress);
  const elapsed = progress
    ? formatElapsed(now - Date.parse(progress.startedAt))
    : null;
  const tokens =
    progress && !progress.thinking && progress.thinkingTokens
      ? `~${progress.thinkingTokens.toLocaleString()} thinking tokens`
      : null;
  const written =
    progress && progress.answerChars > 0
      ? `${progress.answerChars.toLocaleString()} characters written`
      : null;
  return (
    <div className={styles.running}>
      <div className={styles.runningLine} role="status">
        <span className="spinner" aria-hidden="true" />
        <span>{label}</span>
        {elapsed && (
          <span className={styles.meta} aria-label="Elapsed">
            {elapsed}
          </span>
        )}
        {(tokens ?? written) && (
          <span className={styles.meta}>{written ?? tokens}</span>
        )}
        <button type="button" className={styles.stop} onClick={onStop}>
          Stop
        </button>
      </div>
      {progress?.notice && (
        <p className={styles.notice} role="note">
          {progress.notice}
        </p>
      )}
      {progress && progress.thinking && (
        <Thinking
          text={progress.thinking}
          cut={progress.thinkingCut}
          live
          summary="Thinking"
        />
      )}
    </div>
  );
}

/** How close to the end counts as "at the bottom" (px). */
const AT_BOTTOM_PX = 24;

/**
 * The agent's (summarised) thinking: open while it streams, folded once
 * the turn is done; follows the end while the reader is there.
 */
function Thinking({
  text,
  cut,
  live,
  summary,
}: {
  text: string;
  cut: boolean;
  live: boolean;
  summary: string;
}) {
  const [open, setOpen] = useState(live);
  const box = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  useLayoutEffect(() => {
    const el = box.current;
    if (el && open && atBottom.current) el.scrollTop = el.scrollHeight;
  }, [text, open]);
  return (
    <details
      className={styles.thinkingBlock}
      open={open}
      data-live={live ? "true" : undefined}
      onToggle={(e) => setOpen((e.currentTarget as HTMLDetailsElement).open)}
    >
      <summary>{summary}</summary>
      <div
        ref={box}
        className={styles.thinking}
        aria-label="The agent's thinking"
        onScroll={(e) => {
          const el = e.currentTarget;
          atBottom.current =
            el.scrollHeight - el.scrollTop - el.clientHeight < AT_BOTTOM_PX;
        }}
      >
        {cut && <span className={styles.meta}>… </span>}
        {text}
      </div>
    </details>
  );
}
