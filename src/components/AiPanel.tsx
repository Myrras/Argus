/**
 * The AI island right of the review (Iris's preview island): the selected
 * review's findings (`FindingsList`, with "Post selected") over its "look
 * here yourself" checklist (`Checklist`). Open or closed is remembered
 * (`layoutStore.aiPanelOpen`; the header's Findings toggle and the
 * toggle-findings key flip it). Closed, or without a review, or with the AI
 * pass off, it takes no space at all.
 */

import {
  NOT_READ_ONLY_NOTE,
  useAiEnabled,
  useAiRun,
  useAiStore,
} from "../state/aiStore";
import { useLayoutStore } from "../state/layoutStore";
import { useReviewStore } from "../state/reviewStore";
import { withHint } from "../lib/keymap";
import { useKeymap } from "../lib/useKeymap";
import { Checklist } from "./Checklist";
import { FindingsList } from "./FindingsList";
import { AiIcon, AiPanelCloseIcon } from "./icons";
import styles from "./AiPanel.module.css";

export function AiPanel() {
  const reviewId = useReviewStore((s) => s.selectedId);
  const open = useLayoutStore((s) => s.aiPanelOpen);
  const enabled = useAiEnabled(reviewId);
  if (!reviewId || !open || !enabled) return null;
  return <Panel key={reviewId} reviewId={reviewId} />;
}

function Panel({ reviewId }: { reviewId: string }) {
  const close = useLayoutStore((s) => s.toggleAiPanel);
  const run = useAiRun(reviewId);
  const error = useAiStore((s) => s.byReview[reviewId]?.error ?? null);
  const reload = useAiStore((s) => s.reloadOutput);
  const keymap = useKeymap();

  const note =
    run?.status === "running"
      ? "The AI pass is running; what it finds shows here and in the code."
      : run?.queued
        ? "The AI pass waits for a free slot."
        : run?.status === "notStarted"
          ? "No AI pass for this review yet."
          : run?.status === "failed" && run.error !== "cancelled"
            ? `The last AI pass failed: ${run.error ?? "unknown error"}.`
            : null;

  return (
    <aside className={styles.panel} aria-label="AI review">
      <div className={styles.header}>
        <AiIcon className={styles.icon} />
        <h2 className={styles.title}>AI review</h2>
        {run?.round != null && (
          <span className={styles.subtitle}>round {run.round}</span>
        )}
        <button
          type="button"
          className={`ghost ${styles.close}`}
          aria-label="Close the AI panel"
          title={withHint("Close the AI panel", keymap, "toggleFindings")}
          onClick={close}
        >
          <AiPanelCloseIcon />
        </button>
      </div>
      <div className={styles.body}>
        {run?.readOnlyGuaranteed === false && (
          <p className={styles.warning}>{NOT_READ_ONLY_NOTE}</p>
        )}
        {note && <p className={styles.note}>{note}</p>}
        {run && run.droppedFindings > 0 && (
          <p className={styles.note}>
            {run.droppedFindings} more{" "}
            {run.droppedFindings === 1 ? "finding was" : "findings were"} left
            out: not on a file of this change.
          </p>
        )}
        {error && (
          <div className={styles.error} role="alert">
            {error.message}{" "}
            <button type="button" onClick={() => void reload(reviewId)}>
              Retry
            </button>
          </div>
        )}
        <FindingsList reviewId={reviewId} />
        <Checklist reviewId={reviewId} />
        <p className={styles.hint}>
          Nothing is posted until you press Post selected. Verdicts and the
          checklist stay in Argus.
        </p>
      </div>
    </aside>
  );
}
