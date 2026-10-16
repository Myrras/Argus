/**
 * The detail island for the selected review: its header (round state, "Mark
 * round reviewed", the pending comments with "Post all", and "Approve") over
 * one review page (`ReviewView`: the Conversation, then every changed file
 * with its threads inline). Without a selection the island shows the
 * `WelcomeScreen`.
 *
 * Approving is its own explicit action, apart from posting: a confirmation
 * names the head being approved (and warns about pending comments, which it
 * does not post); the backend refuses when the review moved past that head.
 */

import { useState } from "react";

import type { IpcError, ReviewRecord } from "../ipc/types";
import { openExternal } from "../lib/links";
import {
  PROVIDER_LABEL,
  REASON_LABEL,
  STATUS_LABEL,
  reviewRef,
  untouchedLabel,
} from "../lib/reviews";
import { ago, fullTime } from "../lib/time";
import { useCommentStore, useDrafts } from "../state/commentStore";
import { useReviewStore } from "../state/reviewStore";
import { Badge, PlatformBadge } from "./Badge";
import { Modal } from "./Modal";
import { ExternalLinkIcon } from "./icons";
import { ReviewView } from "./ReviewView";
import { WelcomeScreen } from "./WelcomeScreen";
import styles from "./ReviewDetail.module.css";

export function ReviewDetail({ now }: { now: number }) {
  const review = useReviewStore((s) =>
    s.selectedId
      ? (s.reviews.find((r) => r.id === s.selectedId) ?? null)
      : null,
  );
  const refreshing = useReviewStore((s) => s.refreshing);
  const refresh = useReviewStore((s) => s.refresh);

  if (!review) {
    return (
      <section className={styles.island} aria-label="Review detail">
        <WelcomeScreen
          onRefresh={() => void refresh()}
          refreshing={refreshing}
        />
      </section>
    );
  }
  // Keyed so per-review UI state (collapsed sections) resets.
  return <Detail key={review.id} review={review} now={now} />;
}

function Detail({ review: r, now }: { review: ReviewRecord; now: number }) {
  return (
    <section className={styles.island} aria-label="Review detail">
      <ReviewHeader review={r} now={now} />
      <ReviewView review={r} now={now} />
    </section>
  );
}

function ReviewHeader({
  review: r,
  now,
}: {
  review: ReviewRecord;
  now: number;
}) {
  const marking = useReviewStore((s) => s.marking === r.id);
  const markReviewed = useReviewStore((s) => s.markReviewed);
  const canMark = r.status === "needsReview" || r.status === "needsReReview";
  const untouched = untouchedLabel(r.untouchedThreads);
  const drafts = useDrafts(r.id);
  const failed = drafts.filter((d) => d.error).length;
  const posting = useCommentStore((s) => !!s.posting[r.id]);
  const postAll = useCommentStore((s) => s.postAll);
  const [confirming, setConfirming] = useState(false);
  const canApprove = r.state === "open" && !r.approvedAt;

  return (
    <header className={styles.header}>
      <div className={styles.topRow}>
        <PlatformBadge provider={r.provider} />
        <span className={`${styles.ref} mono`}>{reviewRef(r)}</span>
        {r.draft && <Badge>Draft</Badge>}
        <button
          type="button"
          className={`ghost ${styles.open}`}
          onClick={() => openExternal(r.url)}
          title={r.url}
        >
          Open in {PROVIDER_LABEL[r.provider]} <ExternalLinkIcon size={12} />
        </button>
      </div>
      <h1 className={styles.title}>{r.title}</h1>
      <div className={styles.meta}>
        <span>
          by <strong>{r.author}</strong>
        </span>
        {r.updatedAt && (
          <span title={fullTime(r.updatedAt)}>
            updated {ago(r.updatedAt, now)}
          </span>
        )}
        <Badge tone={r.status === "needsReReview" ? "accent" : "neutral"}>
          {STATUS_LABEL[r.status]}
        </Badge>
        <span>Round {r.round}</span>
        {r.state !== "open" && (
          <Badge tone={r.state === "merged" ? "accent" : "neutral"}>
            {r.state === "merged" ? "Merged" : "Closed"}
          </Badge>
        )}
      </div>
      {(r.roundReasons.length > 0 || untouched) && (
        <div className={styles.meta}>
          {r.roundReasons.map((reason) => (
            <Badge key={reason} tone="accent">
              {REASON_LABEL[reason]}
            </Badge>
          ))}
          {untouched && <span className={styles.untouched}>{untouched}</span>}
        </div>
      )}
      <div className={styles.actions}>
        <button
          type="button"
          className="primary"
          disabled={!canMark || marking}
          title={
            canMark
              ? "Remember the current head as reviewed"
              : "Nothing new since your last review"
          }
          onClick={() => void markReviewed(r.id)}
        >
          {marking ? "Marking…" : "Mark round reviewed"}
        </button>
        <button
          type="button"
          disabled={!canApprove}
          title={
            canApprove
              ? `Approve on ${PROVIDER_LABEL[r.provider]} (asks first)`
              : r.approvedAt
                ? "You approved this already"
                : "Only an open review can be approved"
          }
          onClick={() => setConfirming(true)}
        >
          Approve…
        </button>
        {r.lastReviewedAt && (
          <span
            className={styles.lastReviewed}
            title={fullTime(r.lastReviewedAt)}
          >
            Last marked reviewed {ago(r.lastReviewedAt, now)}
          </span>
        )}
        {drafts.length > 0 && (
          <span className={styles.pending} aria-live="polite">
            <span>
              {drafts.length} pending{" "}
              {drafts.length === 1 ? "comment" : "comments"}
              {failed > 0 && (
                <span className={styles.failed}> ({failed} not posted)</span>
              )}
            </span>
            <button
              type="button"
              className="primary"
              disabled={posting}
              title="Post every pending comment of this review now"
              onClick={() => void postAll(r.id)}
            >
              {posting ? "Posting…" : "Post all"}
            </button>
          </span>
        )}
      </div>
      <ApproveModal
        review={r}
        pending={drafts.length}
        open={confirming}
        onClose={() => setConfirming(false)}
      />
    </header>
  );
}

/** Confirm an approval: the head it is for, and what stays unposted. */
export function ApproveModal({
  review: r,
  pending,
  open,
  onClose,
}: {
  review: ReviewRecord;
  pending: number;
  open: boolean;
  onClose: () => void;
}) {
  const approving = useReviewStore((s) => s.approving === r.id);
  const approve = useReviewStore((s) => s.approve);
  const [error, setError] = useState<IpcError | null>(null);
  // The head as shown when the dialog opened: that is what gets approved.
  const [head, setHead] = useState(r.headSha);
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setHead(r.headSha);
      setError(null);
    }
  }
  const close = () => {
    if (!approving) onClose();
  };
  const confirm = async () => {
    const err = await approve(r.id, head);
    if (err) setError(err);
    else onClose();
  };
  return (
    <Modal
      open={open}
      title={`Approve ${reviewRef(r)}`}
      onClose={close}
      footer={
        <>
          <button type="button" disabled={approving} onClick={close}>
            Cancel
          </button>
          <button
            type="button"
            className="primary"
            disabled={approving}
            onClick={() => void confirm()}
          >
            {approving ? "Approving…" : "Approve"}
          </button>
        </>
      }
    >
      <p className={styles.modalText}>
        Approve <strong>{r.title}</strong> on {PROVIDER_LABEL[r.provider]} at
        commit{" "}
        <code className="mono" title={head}>
          {head.slice(0, 7)}
        </code>
        . If the review has newer commits, the approval is refused. Argus never
        merges.
      </p>
      {pending > 0 && (
        <p className={styles.modalWarning} role="note">
          {pending} pending {pending === 1 ? "comment is" : "comments are"} not
          posted. Approving does not post them; use Post all first if they
          should go with it.
        </p>
      )}
      {error && (
        <p className={styles.modalError} role="alert">
          {error.message}
        </p>
      )}
    </Modal>
  );
}
