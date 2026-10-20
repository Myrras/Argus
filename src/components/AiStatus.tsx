/**
 * The AI pass' state in the review header, one compact group: a chip (the
 * sparkle or a spinner, then "6 findings", "Running…", "Failed"; the full
 * state and a failure's reason in its tooltip and for screen readers, and
 * in the tooltip what the last run was given: rules files, review skill,
 * worktree, and anything that did not go as set up), an
 * icon button for Run again / Retry / Cancel, and the toggle of the AI
 * island (panel icon, with the findings count while the chip shows none).
 * Loads the review's run and output when it opens. With the AI pass off it
 * renders nothing (the rest of the review works as before).
 *
 * `AiRunIndicator`: the review list's small spinner while a review's run is
 * queued or running.
 */

import { useEffect } from "react";

import type { ReviewId, ReviewRecord } from "../ipc/types";
import {
  aiChipText,
  aiContextLines,
  aiStatusText,
  skippedSummary,
  skippedText,
} from "../lib/findings";
import { withHint } from "../lib/keymap";
import { useKeymap } from "../lib/useKeymap";
import {
  useAiEnabled,
  useAiRun,
  useAiStore,
  useFindings,
  useSkippedChecklist,
  useSkippedFindings,
} from "../state/aiStore";
import { useLayoutStore } from "../state/layoutStore";
import {
  AiIcon,
  AiPanelCloseIcon,
  AiPanelOpenIcon,
  ErrorIcon,
  RerunIcon,
  RunIcon,
  StopIcon,
} from "./icons";
import styles from "./AiStatus.module.css";

export function AiStatus({ review: r }: { review: ReviewRecord }) {
  const id = r.id;
  const load = useAiStore((s) => s.load);
  useEffect(() => {
    void load(id);
  }, [id, load]);

  const run = useAiRun(id);
  const findings = useFindings(id);
  // Counted from the list the panel shows, so the two always agree.
  const skipped = skippedSummary(
    useSkippedFindings(id),
    useSkippedChecklist(id),
  );
  const starting = useAiStore((s) => !!s.starting[id]);
  const runAi = useAiStore((s) => s.run);
  const cancel = useAiStore((s) => s.cancel);
  const panelOpen = useLayoutStore((s) => s.aiPanelOpen);
  const togglePanel = useLayoutStore((s) => s.toggleAiPanel);
  const keymap = useKeymap();

  if (!run?.enabled) return null;
  const active = findings.filter((f) => !f.dismissed).length;
  const busy = run.status === "running" || run.queued;
  const failed = run.status === "failed" && run.error !== "cancelled";
  const status = busy
    ? run.status === "running"
      ? "running"
      : "queued"
    : run.status;

  // The count shows once: in the chip after a finished run, else (an
  // earlier run's findings while the next one runs or failed) on the toggle.
  const toggleCount = status !== "done" && active > 0;
  const runLabel = failed
    ? "Retry"
    : run.status === "notStarted"
      ? "Run now"
      : "Run again";

  return (
    <span className={styles.ai} role="group" aria-label="AI pass">
      <span
        className={styles.chip}
        data-status={status}
        role="status"
        aria-live="polite"
        title={[
          [
            aiStatusText(run, active),
            run.adapter &&
              `${run.adapter}${run.round !== null ? `, round ${run.round}` : ""}`,
            skippedText(skipped),
          ]
            .filter(Boolean)
            .join(" — "),
          ...aiContextLines(run.context),
        ].join("\n")}
      >
        {busy ? (
          <span className={`spinner ${styles.spinner}`} aria-hidden />
        ) : failed ? (
          <ErrorIcon size={12} />
        ) : (
          <AiIcon size={12} />
        )}
        {/* Screen readers hear the full state, the eye reads the short one. */}
        <span className="visually-hidden">{aiStatusText(run, active)}</span>
        <span className={styles.text} aria-hidden>
          {aiChipText(run, active)}
        </span>
      </span>
      {busy ? (
        <button
          type="button"
          className="iconButton"
          disabled={starting}
          aria-label="Cancel"
          title="Stop the AI pass of this review"
          onClick={() => void cancel(id)}
        >
          <StopIcon />
        </button>
      ) : (
        <button
          type="button"
          className="iconButton"
          disabled={starting}
          aria-label={runLabel}
          title={
            run.status === "notStarted"
              ? "Run now: analyse this review"
              : `${runLabel}: analyse this review again now`
          }
          onClick={() => void runAi(id)}
        >
          {run.status === "notStarted" ? <RunIcon /> : <RerunIcon />}
        </button>
      )}
      <button
        type="button"
        className={`iconButton ${styles.toggle}`}
        aria-pressed={panelOpen}
        aria-label={toggleCount ? `Findings, ${active}` : "Findings"}
        title={withHint(
          panelOpen ? "Hide the AI findings" : "Show the AI findings",
          keymap,
          "toggleFindings",
        )}
        onClick={togglePanel}
      >
        {panelOpen ? <AiPanelCloseIcon /> : <AiPanelOpenIcon />}
        {toggleCount && (
          <span className={styles.count} aria-hidden>
            {active}
          </span>
        )}
      </button>
    </span>
  );
}

export function AiRunIndicator({ reviewId }: { reviewId: ReviewId }) {
  const enabled = useAiEnabled(reviewId);
  const run = useAiRun(reviewId);
  if (!enabled || !run) return null;
  const running = run.status === "running";
  if (!running && !run.queued) return null;
  const label = running ? "AI pass running" : "AI pass queued";
  return (
    <span
      className={styles.indicator}
      data-status={running ? "running" : "queued"}
      role="img"
      aria-label={label}
      title={label}
    >
      {running ? (
        <span className="spinner" aria-hidden />
      ) : (
        <AiIcon size={11} />
      )}
    </span>
  );
}
