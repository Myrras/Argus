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
 *
 * "Run after submitting" lists the user's submit scripts set for this
 * repository and the chosen verdict (`lib/submitScripts`), ticked by their
 * "Ticked by default" until the user ticks otherwise; the ticked ids go
 * with the submit, and the backend runs them only once it succeeded.
 * Without any script configured, one quiet line points to Settings →
 * Scripts.
 */

import { useState } from "react";

import type {
  IpcError,
  ReviewRecord,
  ReviewVerdict,
  SubmitScript,
} from "../ipc/types";
import {
  PROVIDER_LABEL,
  VERDICT_LABEL,
  needsSummary,
  reviewRef,
  verdictsFor,
} from "../lib/reviews";
import {
  isTicked,
  repoKey,
  scriptName,
  scriptsFor,
} from "../lib/submitScripts";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import {
  postableDrafts,
  useCommentStore,
  useDrafts,
  type Draft,
} from "../state/commentStore";
import { useReviewStore } from "../state/reviewStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { Markdown } from "./Markdown";
import { Modal } from "./Modal";
import styles from "./SubmitReviewModal.module.css";

const NO_VERDICT_LABEL = "Submit review";
/** A stable empty list, so the selector does not re-render on every read. */
const NO_SCRIPTS: SubmitScript[] = [];
const POSTING_LABEL = "Posting comments…";
const SUBMITTING_LABEL = "Submitting…";

/**
 * A button label as wide as the widest of `labels`: all of them share one
 * grid cell, only `current` is visible and named. A footer is right-aligned,
 * so a label that changes width would shift the buttons beside it, in the
 * same render that dims them disabled; WKWebView can leave the old paint of
 * a button that moves while it changes like that (a stale, enabled-looking
 * "Cancel" beside the real one). Nothing moves, nothing is left behind.
 */
function StableLabel({
  labels,
  current,
}: {
  labels: readonly string[];
  current: string;
}) {
  return (
    <span className={styles.stableLabel}>
      {[...new Set([...labels, current])].map((l) => (
        <span
          key={l}
          data-current={l === current}
          aria-hidden={l === current ? undefined : true}
        >
          {l}
        </span>
      ))}
    </span>
  );
}

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
  // The user's own ticks per script id (kept when the verdict changes);
  // a script without one follows its "Ticked by default".
  const [ticks, setTicks] = useState<Record<string, boolean>>({});
  const allScripts = useBackendSettingsStore(
    (s) => s.settings?.submitScripts ?? NO_SCRIPTS,
  );
  const offered = scriptsFor(allScripts, verdict, repoKey(r));
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
      setTicks({});
    }
  }

  const busy = submitting || posting;
  const missingSummary = needsSummary(r, verdict) && summary.trim() === "";
  const close = () => {
    if (!busy) onClose();
  };
  const manageScripts = () => {
    if (busy) return;
    onClose();
    useSettingsRequestStore.getState().open("scripts");
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
    const scriptIds = offered
      .filter((s) => isTicked(s, ticks))
      .map((s) => s.id);
    const err = await submit(r.id, head, verdict, summary, scriptIds);
    if (err) setError(err);
    else {
      setSummary("");
      onClose();
    }
  };

  const choices = verdictsFor(r.provider);
  const label = posting
    ? POSTING_LABEL
    : submitting
      ? SUBMITTING_LABEL
      : verdict
        ? VERDICT_LABEL[verdict].button
        : NO_VERDICT_LABEL;
  // Every label the primary can show: it keeps the widest one's width, so
  // neither button moves when the verdict or the busy state changes it.
  const labels = [
    NO_VERDICT_LABEL,
    ...choices.map((v) => VERDICT_LABEL[v].button),
    POSTING_LABEL,
    SUBMITTING_LABEL,
  ];
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
            <StableLabel labels={labels} current={label} />
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

        {allScripts.length === 0 ? (
          <p className={styles.scriptsHint}>
            Want something to happen after you submit?{" "}
            <button
              type="button"
              className={`ghost ${styles.link}`}
              disabled={busy}
              onClick={manageScripts}
            >
              Add a submit script
            </button>
          </p>
        ) : (
          offered.length > 0 && (
            <fieldset className={styles.scripts}>
              <legend className={styles.legend}>Run after submitting</legend>
              {offered.map((s) => (
                <label key={s.id} className={styles.drafts}>
                  <input
                    type="checkbox"
                    checked={isTicked(s, ticks)}
                    disabled={busy}
                    onChange={(e) =>
                      setTicks((t) => ({ ...t, [s.id]: e.target.checked }))
                    }
                  />
                  {scriptName(s)}
                </label>
              ))}
              <p className={styles.scriptsHint}>
                Runs on your machine after the review is submitted.{" "}
                <button
                  type="button"
                  className={`ghost ${styles.link}`}
                  disabled={busy}
                  onClick={manageScripts}
                >
                  Manage scripts
                </button>
              </p>
            </fieldset>
          )
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
