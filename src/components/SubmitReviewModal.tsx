/**
 * Submit review: the user's verdict on the platform (Approve, Request
 * changes, and on GitHub Comment), with an optional markdown summary
 * (Write / Preview), pinned to the head shown when the dialog opened.
 *
 * "Post N pending comments with it" (on by default when there are drafts)
 * posts the drafts first through the usual post path; if any is refused,
 * nothing is submitted and the dialog lists each refused draft with the
 * platform's reason. Only the dialog's own button writes; the backend
 * refuses when the review moved past the head shown.
 */

import { useState } from "react";

import type { IpcError, ReviewRecord, ReviewVerdict } from "../ipc/types";
import {
  PROVIDER_LABEL,
  VERDICT_LABEL,
  needsSummary,
  reviewRef,
  verdictsFor,
} from "../lib/reviews";
import {
  postableDrafts,
  useCommentStore,
  useDrafts,
  type Draft,
} from "../state/commentStore";
import { useReviewStore } from "../state/reviewStore";
import { Markdown } from "./Markdown";
import { Modal } from "./Modal";
import styles from "./SubmitReviewModal.module.css";

export function SubmitReviewModal({
  review: r,
  open,
  onClose,
  verdict: initialVerdict = null,
}: {
  review: ReviewRecord;
  open: boolean;
  onClose: () => void;
  /** Preselected (the palette's Approve…). */
  verdict?: ReviewVerdict | null;
}) {
  const submitting = useReviewStore((s) => s.approving === r.id);
  const submit = useReviewStore((s) => s.submit);
  const posting = useCommentStore((s) => !!s.posting[r.id]);
  const postAll = useCommentStore((s) => s.postAll);
  const drafts = useDrafts(r.id);
  const pending = useCommentStore((s) => postableDrafts(s, r.id).length);

  const [verdict, setVerdict] = useState<ReviewVerdict | null>(initialVerdict);
  const [summary, setSummary] = useState("");
  const [preview, setPreview] = useState(false);
  const [withDrafts, setWithDrafts] = useState(pending > 0);
  const [error, setError] = useState<IpcError | null>(null);
  const [refused, setRefused] = useState<Draft[] | null>(null);
  // The head as shown when the dialog opened: that is what gets judged.
  const [head, setHead] = useState(r.headSha);
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setHead(r.headSha);
      setVerdict(initialVerdict);
      setWithDrafts(pending > 0);
      setPreview(false);
      setError(null);
      setRefused(null);
    }
  }

  const busy = submitting || posting;
  const missingSummary = needsSummary(r, verdict) && summary.trim() === "";
  const close = () => {
    if (!busy) onClose();
  };
  const confirm = async () => {
    if (!verdict || missingSummary || busy) return;
    setError(null);
    setRefused(null);
    if (withDrafts && pending > 0) {
      const outcome = await postAll(r.id);
      if (outcome && (outcome.error || outcome.failed > 0)) {
        // Nothing is submitted over a comment that did not go out.
        if (outcome.error) setError(outcome.error);
        setRefused(
          useCommentStore.getState().drafts[r.id]?.filter((d) => d.error) ?? [],
        );
        return;
      }
    }
    const err = await submit(r.id, head, verdict, summary);
    if (err) setError(err);
    else {
      setSummary("");
      onClose();
    }
  };

  const choices = verdictsFor(r.provider);
  const label = verdict ? VERDICT_LABEL[verdict].button : "Submit review";
  return (
    <Modal
      open={open}
      title={`Submit review — ${reviewRef(r)}`}
      onClose={close}
      width="min(560px, 94vw)"
      footer={
        <>
          <button type="button" disabled={busy} onClick={close}>
            Cancel
          </button>
          <button
            type="button"
            className="primary"
            disabled={!verdict || missingSummary || busy}
            title={
              !verdict
                ? "Choose a verdict first"
                : missingSummary
                  ? `${PROVIDER_LABEL[r.provider]} needs a summary for this`
                  : undefined
            }
            onClick={() => void confirm()}
          >
            {posting ? "Posting comments…" : submitting ? "Submitting…" : label}
          </button>
        </>
      }
    >
      <div className={styles.body}>
        <fieldset className={styles.verdicts}>
          <legend className={styles.legend}>Your review</legend>
          {choices.map((v) => (
            <label
              key={v}
              className={styles.verdict}
              data-checked={verdict === v}
            >
              <input
                type="radio"
                name={`verdict-${r.id}`}
                value={v}
                checked={verdict === v}
                disabled={busy}
                onChange={() => setVerdict(v)}
              />
              <span className={styles.verdictText}>
                <span className={styles.verdictName}>
                  {VERDICT_LABEL[v].option}
                </span>
                <span className={styles.verdictHint}>
                  {VERDICT_LABEL[v].hint}
                </span>
              </span>
            </label>
          ))}
        </fieldset>

        <div className={styles.summary}>
          <div className={styles.tabs} role="tablist" aria-label="Summary">
            <button
              type="button"
              role="tab"
              aria-selected={!preview}
              className={styles.tab}
              onClick={() => setPreview(false)}
            >
              Write
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={preview}
              className={styles.tab}
              onClick={() => setPreview(true)}
            >
              Preview
            </button>
            <span className={styles.optional}>
              {needsSummary(r, verdict) ? "Required" : "Optional"}
            </span>
          </div>
          {preview ? (
            <div className={styles.preview}>
              {summary.trim() ? (
                <Markdown>{summary}</Markdown>
              ) : (
                <p className={styles.nothing}>Nothing to preview.</p>
              )}
            </div>
          ) : (
            <textarea
              className={styles.field}
              aria-label="Summary"
              placeholder="Summary (markdown)"
              value={summary}
              disabled={busy}
              onChange={(e) => setSummary(e.target.value)}
            />
          )}
        </div>

        {drafts.length > 0 && (
          <label className={styles.drafts}>
            <input
              type="checkbox"
              checked={withDrafts}
              disabled={busy || pending === 0}
              onChange={(e) => setWithDrafts(e.target.checked)}
            />
            Post {pending} pending {pending === 1 ? "comment" : "comments"} with
            it
          </label>
        )}
        {drafts.length > 0 && !withDrafts && (
          <p className={styles.warning} role="note">
            {drafts.length} pending{" "}
            {drafts.length === 1 ? "comment stays" : "comments stay"} unposted.
          </p>
        )}

        <p className={styles.text}>
          On {PROVIDER_LABEL[r.provider]} at commit{" "}
          <code className="mono" title={head}>
            {head.slice(0, 7)}
          </code>
          . If the review has newer commits, it is refused. Argus never merges.
        </p>

        {refused && (
          <div className={styles.error} role="alert">
            <p className={styles.errorTitle}>
              Not submitted:{" "}
              {refused.length > 0
                ? `${refused.length} pending ${refused.length === 1 ? "comment was" : "comments were"} not posted.`
                : "the pending comments could not be posted."}
            </p>
            {refused.length > 0 && (
              <ul className={styles.refused}>
                {refused.map((d) => (
                  <li key={d.id}>
                    <span className="mono">
                      {d.path}:{d.line}
                    </span>{" "}
                    — {d.error?.message}
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}
        {error && (
          <p className={styles.error} role="alert">
            {error.message}
          </p>
        )}
      </div>
    </Modal>
  );
}
