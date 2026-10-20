/**
 * The dialogs before a write to a platform, rendered once (`App`): the
 * Submit review dialog and the palette's "Post all" confirmation. Their one
 * owner is `overlayStore.confirm`; every entry point (the header's button,
 * ⇧S, the palette, the list's menu) only asks for one, so a second request
 * takes over the open dialog instead of stacking another on top.
 */

import type { ReviewId } from "../ipc/types";
import { PROVIDER_LABEL, reviewRef } from "../lib/reviews";
import { useCommentStore, usePostableCount } from "../state/commentStore";
import { useOverlayStore } from "../state/overlayStore";
import { useReviewStore } from "../state/reviewStore";
import { Modal } from "./Modal";
import { SubmitReviewModal } from "./SubmitReviewModal";
import styles from "./ReviewDialogs.module.css";

/** Confirm "Post all" from the palette: the header's button, asked first. */
function PostAllModal({
  reviewId,
  onClose,
}: {
  reviewId: ReviewId;
  onClose: () => void;
}) {
  const review = useReviewStore((s) =>
    s.reviews.find((r) => r.id === reviewId),
  );
  // What Post all sends: an emptied edit's draft is left out.
  const n = usePostableCount(reviewId);
  const postAll = useCommentStore((s) => s.postAll);
  return (
    <Modal
      open
      title={`Post ${n} pending ${n === 1 ? "comment" : "comments"}`}
      onClose={onClose}
      footer={
        <>
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="primary"
            disabled={n === 0}
            onClick={() => {
              onClose();
              void postAll(reviewId);
            }}
          >
            Post all
          </button>
        </>
      }
    >
      <p className={styles.confirmText}>
        Posts every pending comment of{" "}
        <strong>{review ? reviewRef(review) : "this review"}</strong>
        {review ? ` on ${PROVIDER_LABEL[review.provider]}` : ""} now, one by
        one. A comment the platform refuses stays pending with the reason.
      </p>
    </Modal>
  );
}

export function ReviewDialogs() {
  const confirm = useOverlayStore((s) => s.confirm);
  const closeConfirm = useOverlayStore((s) => s.closeConfirm);
  const review = useReviewStore((s) =>
    confirm ? s.reviews.find((r) => r.id === confirm.reviewId) : undefined,
  );
  if (!confirm || !review) return null;
  return confirm.kind === "submit" ? (
    <SubmitReviewModal
      // A request for another review or verdict starts a fresh dialog.
      key={`${review.id}:${confirm.verdict ?? ""}`}
      review={review}
      verdict={confirm.verdict ?? null}
      open
      onClose={closeConfirm}
    />
  ) : (
    <PostAllModal reviewId={review.id} onClose={closeConfirm} />
  );
}
