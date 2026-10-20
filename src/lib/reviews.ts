/**
 * Pure presentation helpers over the review wire types: tabs, labels and
 * thread ordering. No IPC, no state.
 */

import type {
  MyReviewState,
  ProviderId,
  ReviewRecord,
  ReviewStatus,
  ReviewVerdict,
  RoundReason,
  StoredThread,
} from "../ipc/types";

/** The four list tabs, in display order (each is a `ReviewStatus`). */
export type ReviewTab = Exclude<ReviewStatus, "approved">;

export const TABS: readonly { id: ReviewTab; label: string }[] = [
  { id: "needsReview", label: "Needs review" },
  { id: "needsReReview", label: "Needs re-review" },
  { id: "waitingOnAuthor", label: "Waiting on author" },
  { id: "closedWithoutApproval", label: "Closed without my approval" },
];

/** What an empty tab says. */
export const EMPTY_TAB: Record<ReviewTab, string> = {
  needsReview: "Nothing waiting for a first review.",
  needsReReview: "No reviews need another look.",
  waitingOnAuthor: "Nothing is waiting on an author.",
  closedWithoutApproval: "Nothing was merged or closed without your approval.",
};

/**
 * The reviews of `tab`. `lingering` (review id → tab) keeps a review just
 * approved from Argus in the tab it was in for a moment ("Approved ✓");
 * one the platform no longer shows approved is in its own tab as usual.
 */
export function reviewsInTab(
  reviews: readonly ReviewRecord[],
  tab: ReviewTab,
  lingering: Partial<Record<string, ReviewTab>> = {},
): ReviewRecord[] {
  return reviews.filter(
    (r) =>
      r.status === tab || (r.status === "approved" && lingering[r.id] === tab),
  );
}

/** What a submitted verdict did, for the toast ("Approved owner/repo#12"). */
export const VERDICT_DONE: Record<ReviewVerdict, string> = {
  approve: "Approved",
  requestChanges: "Requested changes on",
  comment: "Reviewed",
};

/** The verdicts a platform's Submit review offers, in order. */
export function verdictsFor(provider: ProviderId): ReviewVerdict[] {
  return provider === "github"
    ? ["approve", "requestChanges", "comment"]
    : ["approve", "requestChanges"];
}

/** Whether `verdict` needs a summary on `r`'s platform (GitHub's API). */
export function needsSummary(
  r: Pick<ReviewRecord, "provider">,
  verdict: ReviewVerdict | null,
): boolean {
  return (
    r.provider === "github" &&
    (verdict === "requestChanges" || verdict === "comment")
  );
}

/** Each verdict's option label and its button. */
export const VERDICT_LABEL: Record<
  ReviewVerdict,
  { option: string; button: string; hint: string }
> = {
  approve: {
    option: "Approve",
    button: "Approve",
    hint: "Approve the changes. Leaves your lists once done.",
  },
  requestChanges: {
    option: "Request changes",
    button: "Request changes",
    hint: "The author's move: back in your lists after new commits or a reply.",
  },
  comment: {
    option: "Comment",
    button: "Submit comment",
    hint: "Feedback without a verdict (GitHub). A summary is required.",
  },
};

/** The user's own review state on the platform, for the header. */
export const MY_STATE_LABEL: Record<MyReviewState, string | null> = {
  awaitingReview: "Awaiting your review",
  reviewStarted: "Review started",
  requestedChanges: "You requested changes",
  commented: "You reviewed",
  approved: "You approved",
  notReviewer: null,
};

export function tabCounts(
  reviews: readonly ReviewRecord[],
  lingering: Partial<Record<string, ReviewTab>> = {},
): Record<ReviewTab, number> {
  const counts: Record<ReviewTab, number> = {
    needsReview: 0,
    needsReReview: 0,
    waitingOnAuthor: 0,
    closedWithoutApproval: 0,
  };
  for (const r of reviews) {
    if (r.status !== "approved") counts[r.status] += 1;
    else if (lingering[r.id]) counts[lingering[r.id]!] += 1;
  }
  return counts;
}

export const REASON_LABEL: Record<RoundReason, string> = {
  newCommits: "New commits",
  authorReplied: "Author replied",
  reRequested: "Re-requested",
};

export const STATUS_LABEL: Record<ReviewStatus, string> = {
  needsReview: "Needs review",
  needsReReview: "Needs re-review",
  waitingOnAuthor: "Waiting on author",
  closedWithoutApproval: "Closed without approval",
  approved: "Approved",
};

export const PROVIDER_LABEL: Record<ProviderId, string> = {
  github: "GitHub",
  gitlab: "GitLab",
};

/** `owner/repo#12` on GitHub, `group/project!12` on GitLab. */
export function reviewRef(
  r: Pick<ReviewRecord, "provider" | "repo" | "number">,
) {
  return `${r.repo}${r.provider === "gitlab" ? "!" : "#"}${r.number}`;
}

/** "N of your threads untouched", or null when none are. */
export function untouchedLabel(n: number): string | null {
  if (n <= 0) return null;
  return `${n} of your threads untouched`;
}

/**
 * Where a thread sits: `path:line`, `path:start–end` for a range, just the
 * path for a file-level comment, or MR-/PR-level without an anchor.
 */
export function threadLocation(t: StoredThread, provider: ProviderId): string {
  const a = t.anchor;
  if (!a) return provider === "gitlab" ? "MR-level" : "PR-level";
  const path =
    (a.side === "old" ? a.pathOld : a.pathNew) ?? a.pathNew ?? a.pathOld;
  const line =
    (a.side === "old" ? a.lineOld : a.lineNew) ?? a.lineNew ?? a.lineOld;
  if (!path) return provider === "gitlab" ? "MR-level" : "PR-level";
  if (line === null) return path;
  if (a.startLine !== null && a.startLine !== line) {
    return `${path}:${a.startLine}–${line}`;
  }
  return `${path}:${line}`;
}

/** The user's threads and everyone else's, each oldest first. */
export function splitThreads(threads: readonly StoredThread[]): {
  mine: StoredThread[];
  others: StoredThread[];
} {
  const byTime = [...threads].sort((a, b) =>
    a.createdAt < b.createdAt ? -1 : a.createdAt > b.createdAt ? 1 : 0,
  );
  return {
    mine: byTime.filter((t) => t.mine),
    others: byTime.filter((t) => !t.mine),
  };
}

/**
 * Whether "since my last review" applies: the user reviewed a head (Argus'
 * watermark) and the head moved on since.
 */
export function canDiffSinceLastReview(
  r: Pick<ReviewRecord, "lastReviewedSha" | "headSha">,
): boolean {
  return !!r.lastReviewedSha && r.lastReviewedSha !== r.headSha;
}
