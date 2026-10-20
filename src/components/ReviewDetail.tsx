/**
 * The detail island for the selected review: its header over one review
 * page (`ReviewView`: the Conversation, then every changed file with its
 * threads inline). Without a selection the island shows the
 * `WelcomeScreen`.
 *
 * The header reads top-down: the title, with the actions top right (the AI
 * group, pending comments with "Post all", then the review's verdict slot:
 * "Submit review…"); where the review lives and who wrote it; then its
 * status with the round, the user's review state on the platform, the
 * reasons and what is untouched. Under a narrow island the actions move
 * below (container query).
 *
 * Submitting a review (Approve / Request changes / on GitHub Comment) is
 * its own explicit action in `SubmitReviewModal`: it names the head being
 * judged and can post the pending comments first; the backend refuses
 * when the review moved past that head. The header's button only asks
 * `overlayStore` for it, as ⇧S and the palette do, so there is only ever
 * one (`ReviewDialogs`).
 */

import type { ReviewRecord } from "../ipc/types";
import { openExternal } from "../lib/links";
import {
  MY_STATE_LABEL,
  PROVIDER_LABEL,
  REASON_LABEL,
  STATUS_LABEL,
  reviewRef,
  untouchedLabel,
} from "../lib/reviews";
import { ago, fullTime } from "../lib/time";
import { useCommentStore, useDrafts } from "../state/commentStore";
import { useOverlayStore } from "../state/overlayStore";
import { useReviewStore } from "../state/reviewStore";
import { AiStatus } from "./AiStatus";
import { Badge, PlatformBadge } from "./Badge";
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
  const submitting = useReviewStore((s) => s.approving === r.id);
  const untouched = untouchedLabel(r.untouchedThreads);
  const drafts = useDrafts(r.id);
  const failed = drafts.filter((d) => d.error).length;
  const posting = useCommentStore((s) => !!s.posting[r.id]);
  const postAll = useCommentStore((s) => s.postAll);
  const ask = useOverlayStore((s) => s.ask);
  const canSubmit = r.state === "open";
  const myState = MY_STATE_LABEL[r.myReviewState];

  return (
    <header className={styles.header}>
      <div className={styles.grid}>
        <h1 className={styles.title}>{r.title}</h1>
        <div className={styles.actions}>
          <AiStatus review={r} />
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
                disabled={posting}
                title="Post every pending comment of this review now"
                onClick={() => void postAll(r.id)}
              >
                {posting ? "Posting…" : "Post all"}
              </button>
            </span>
          )}
          {/* The review's own verdict: one slot, top right. */}
          <div className={styles.submit} data-slot="review-submit">
            <button
              type="button"
              className="primary"
              disabled={!canSubmit || submitting}
              title={
                canSubmit
                  ? `Approve or request changes on ${PROVIDER_LABEL[r.provider]} (opens a dialog)`
                  : "Only an open review can be reviewed"
              }
              onClick={() => ask({ kind: "submit", reviewId: r.id })}
            >
              {submitting ? "Submitting…" : "Submit review…"}
            </button>
          </div>
        </div>
        <div className={styles.meta}>
          <PlatformBadge provider={r.provider} />
          <span className={styles.ref}>
            <span className={`${styles.refText} mono`}>{reviewRef(r)}</span>
            <button
              type="button"
              className={`iconButton ${styles.open}`}
              onClick={() => openExternal(r.url)}
              aria-label={`Open in ${PROVIDER_LABEL[r.provider]}`}
              title={`Open in ${PROVIDER_LABEL[r.provider]}: ${r.url}`}
            >
              <ExternalLinkIcon />
            </button>
          </span>
          {r.draft && <Badge>Draft</Badge>}
          {r.state !== "open" && (
            <Badge tone={r.state === "merged" ? "accent" : "neutral"}>
              {r.state === "merged" ? "Merged" : "Closed"}
            </Badge>
          )}
          <span>
            by <strong>{r.author}</strong>
          </span>
          {r.updatedAt && (
            <span title={fullTime(r.updatedAt)}>
              updated {ago(r.updatedAt, now)}
            </span>
          )}
        </div>
        <div className={styles.status}>
          {/* The status and its round read as one: "Needs re-review · R2". */}
          <span
            className={styles.statusPill}
            data-tone={r.status === "needsReReview" ? "accent" : "neutral"}
          >
            <span>{STATUS_LABEL[r.status]}</span>
            <span className={styles.round} title={`Round ${r.round}`}>
              <span aria-hidden>R{r.round}</span>
              <span className="visually-hidden">Round {r.round}</span>
            </span>
          </span>
          {myState && (
            <span
              className={styles.myState}
              data-state={r.myReviewState}
              title={`Your review state on ${PROVIDER_LABEL[r.provider]}`}
            >
              {myState}
            </span>
          )}
          {r.roundReasons.map((reason) => (
            <Badge key={reason} tone="accent">
              {REASON_LABEL[reason]}
            </Badge>
          ))}
          {untouched && <span className={styles.untouched}>{untouched}</span>}
          {r.lastReviewedAt && (
            <span
              className={styles.lastReviewed}
              title={fullTime(r.lastReviewedAt)}
            >
              Last reviewed in Argus {ago(r.lastReviewedAt, now)}
            </span>
          )}
        </div>
      </div>
    </header>
  );
}
