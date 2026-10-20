/**
 * The AI pass' state in the review header: a compact chip ("AI: running…",
 * "AI: done · 6 findings", "AI: failed — <reason>", "AI: queued"), Run again
 * / Retry / Cancel, and the toggle of the AI island. Loads the review's run
 * and output when it opens. With the AI pass off it renders nothing (the
 * rest of the review works as before).
 *
 * `AiRunIndicator`: the review list's small spinner while a review's run is
 * queued or running.
 */

import { useEffect } from "react";

import type { ReviewId, ReviewRecord } from "../ipc/types";
import { aiStatusText, skippedSummary, skippedText } from "../lib/findings";
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
import { AiIcon } from "./icons";
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

  return (
    <span className={styles.ai}>
      <span
        className={styles.chip}
        data-status={status}
        role="status"
        aria-live="polite"
        title={[
          aiStatusText(run, active),
          run.adapter &&
            `${run.adapter}${run.round !== null ? `, round ${run.round}` : ""}`,
          skippedText(skipped),
        ]
          .filter(Boolean)
          .join(" — ")}
      >
        {busy ? (
          <span className={`spinner ${styles.spinner}`} aria-hidden />
        ) : (
          <AiIcon size={12} />
        )}
        <span className={styles.text}>{aiStatusText(run, active)}</span>
      </span>
      {busy ? (
        <button
          type="button"
          className="ghost"
          disabled={starting}
          title="Stop the AI pass of this review"
          onClick={() => void cancel(id)}
        >
          Cancel
        </button>
      ) : (
        <button
          type="button"
          className="ghost"
          disabled={starting}
          title="Analyse this review again now"
          onClick={() => void runAi(id)}
        >
          {failed
            ? "Retry"
            : run.status === "notStarted"
              ? "Run now"
              : "Run again"}
        </button>
      )}
      <button
        type="button"
        className={`ghost ${styles.toggle}`}
        aria-pressed={panelOpen}
        title={withHint(
          panelOpen ? "Hide the AI findings" : "Show the AI findings",
          keymap,
          "toggleFindings",
        )}
        onClick={togglePanel}
      >
        Findings
        {active > 0 && <span className={styles.count}>{active}</span>}
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
