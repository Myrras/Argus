/**
 * The AI island's findings: grouped by severity (high → medium → low), then
 * path and line. Each row has its "select for posting" checkbox, the title
 * (a click scrolls the code view to the finding), where it is, "from round
 * N" for an earlier round's unposted one, "Edited", a link once posted, and
 * the reason when its last post failed, and the comment it relates to or
 * (shown anyway) repeats. Dismissed findings stay hidden behind the "Show
 * dismissed" toggle.
 *
 * The header's "Post selected (N)" is the only way findings reach the
 * platform: each posted one gets its link, a failed one stays selected with
 * its reason, and the rest are posted regardless.
 *
 * `SkippedFindings`: the findings and checklist items the run set aside
 * because the review already makes their point, under "Skipped as already
 * raised (N, k high)", folded unless a high one is among them: each with a
 * link to the comment it repeats and "Show anyway" (local; it then lists as
 * a normal finding or item). Never posted from there.
 */

import { useState, type ReactNode } from "react";

import type {
  ChecklistItem,
  Finding,
  FindingId,
  RepeatOf,
  ReviewId,
  StoredThread,
  ThreadId,
} from "../ipc/types";
import {
  RELATED_HINT,
  REPEAT_REASON_HINT,
  SEVERITY_LABEL,
  SHOWN_ANYWAY_HINT,
  SEVERITY_TONE,
  findingLocation,
  fromEarlierRound,
  groupBySeverity,
  postable,
  relatedLabel,
  repeatTarget,
  skippedHeader,
  skippedSummary,
  sortFindings,
} from "../lib/findings";
import {
  openPosted,
  useAiStore,
  useCurrentRound,
  useFindings,
  useSkippedChecklist,
  useSkippedFindings,
} from "../state/aiStore";
import { useCodeViewStore } from "../state/codeViewStore";
import { pauseScrollAnchor } from "../lib/scrollAnchor";
import { focusThread } from "../state/focusStore";
import { useThreadStore } from "../state/threadStore";
import { Badge } from "./Badge";
import { CaretIcon, ExternalLinkIcon } from "./icons";
import styles from "./FindingsList.module.css";

/** Select the finding's file and scroll to the finding once it rendered. */
function scrollToFinding(reviewId: ReviewId, f: Finding) {
  useAiStore.getState().revealFinding(f);
  useCodeViewStore.getState().select(reviewId, f.path);
  let tries = 0;
  const tick = () => {
    const el = document.getElementById(`finding-${f.id}`);
    if (el) {
      pauseScrollAnchor();
      el.scrollIntoView?.({ block: "center" });
      return;
    }
    // The card loads its diff once selected; give it a moment.
    tries += 1;
    if (tries < 40) setTimeout(tick, 50);
  };
  // After the card's own scroll to its top.
  setTimeout(tick, 60);
}

export function FindingsList({ reviewId }: { reviewId: ReviewId }) {
  const findings = useFindings(reviewId);
  const ui = useAiStore((s) => s.ui);
  const posting = useAiStore((s) => !!s.posting[reviewId]);
  const postSelected = useAiStore((s) => s.postSelected);
  const showDismissed = useAiStore((s) => s.showDismissed);
  const setShowDismissed = useAiStore((s) => s.setShowDismissed);

  const dismissed = findings.filter((f) => f.dismissed).length;
  const shown = showDismissed ? findings : findings.filter((f) => !f.dismissed);
  const selected = findings.filter((f) => postable(f) && ui[f.id]?.selected);
  const groups = groupBySeverity(shown);

  return (
    <section className={styles.section} aria-label="AI findings">
      <div className={styles.toolbar}>
        <h3 className={styles.title}>
          Findings{" "}
          <span className={styles.count}>{findings.length - dismissed}</span>
        </h3>
        <button
          type="button"
          className="primary"
          disabled={selected.length === 0 || posting}
          title="Post the selected findings as line comments now"
          onClick={() => void postSelected(reviewId)}
        >
          {posting ? "Posting…" : `Post selected (${selected.length})`}
        </button>
      </div>
      {dismissed > 0 && (
        <button
          type="button"
          className={`ghost ${styles.dismissedToggle}`}
          aria-pressed={showDismissed}
          onClick={() => setShowDismissed(!showDismissed)}
        >
          {showDismissed ? "Hide dismissed" : `Show dismissed (${dismissed})`}
        </button>
      )}
      {groups.length === 0 ? (
        <p className={styles.empty}>No findings.</p>
      ) : (
        groups.map((g) => (
          <div key={g.severity} className={styles.group}>
            <h4 className={styles.groupTitle} data-severity={g.severity}>
              {SEVERITY_LABEL[g.severity]}{" "}
              <span className={styles.count}>{g.findings.length}</span>
            </h4>
            <ul className={styles.list}>
              {g.findings.map((f) => (
                <FindingRow key={f.id} reviewId={reviewId} finding={f} />
              ))}
            </ul>
          </div>
        ))
      )}
    </section>
  );
}

function FindingRow({
  reviewId,
  finding: f,
}: {
  reviewId: ReviewId;
  finding: Finding;
}) {
  const ui = useAiStore((s) => s.ui[f.id]);
  const focused = useAiStore((s) => s.focused === f.id);
  const posting = useAiStore((s) => !!s.posting[reviewId]);
  const setSelected = useAiStore((s) => s.setSelected);
  const dismiss = useAiStore((s) => s.dismiss);
  const round = useCurrentRound(reviewId);
  const threads = useThreadStore(
    (s) => s.byReview[reviewId]?.threads ?? NO_THREADS,
  );
  const canPost = postable(f);
  const related = relatedLabel(f, threads);
  const pointed = f.repeatOf ?? f.relatedTo[0] ?? null;
  return (
    <li
      className={styles.row}
      data-severity={f.severity}
      data-focused={focused}
      data-dismissed={f.dismissed}
      data-posted={!!f.postedAt}
      aria-label={`Finding: ${f.title}`}
    >
      <span className={styles.checkCell}>
        {canPost && (
          <input
            type="checkbox"
            checked={!!ui?.selected}
            disabled={posting}
            aria-label={`Select "${f.title}" for posting`}
            onChange={(e) => setSelected(f.id, e.target.checked)}
          />
        )}
      </span>
      <span className={styles.main}>
        <button
          type="button"
          className={styles.open}
          title="Show it in the code"
          onClick={() => scrollToFinding(reviewId, f)}
        >
          <span className={styles.rowTitle}>{f.title}</span>
          <span className={`${styles.where} mono`}>{findingLocation(f)}</span>
        </button>
        <span className={styles.tags}>
          <Badge tone={SEVERITY_TONE[f.severity]}>
            {SEVERITY_LABEL[f.severity]}
          </Badge>
          {fromEarlierRound(f, round) && <Badge>from round {f.round}</Badge>}
          {ui?.edit != null && !f.postedAt && (
            <Badge tone="accent">Edited</Badge>
          )}
          {(related || f.repeatOf) && pointed && (
            <PointLink
              reviewId={reviewId}
              threadId={pointed.threadId}
              findingId={pointed.findingId}
              title={f.repeatOf ? SHOWN_ANYWAY_HINT : RELATED_HINT}
            >
              {related ?? `repeats ${repeatTarget(f, threads)}`}
            </PointLink>
          )}
          {f.postedAt && (
            <button
              type="button"
              className={`ghost ${styles.link}`}
              title={f.postedUrl ?? undefined}
              disabled={!f.postedUrl}
              onClick={() => openPosted(f)}
            >
              Posted <ExternalLinkIcon size={11} />
            </button>
          )}
          {f.dismissed && (
            <button
              type="button"
              className={`ghost ${styles.link}`}
              onClick={() => void dismiss(f, false)}
            >
              Undismiss
            </button>
          )}
        </span>
        {ui?.error && (
          <span className={styles.error} role="alert">
            Not posted: {ui.error.message}
          </span>
        )}
      </span>
    </li>
  );
}

const NO_THREADS: StoredThread[] = [];

/**
 * A link to a point of the review: focuses its thread, or scrolls to the
 * posted finding. Disabled when neither is loaded.
 */
export function PointLink({
  reviewId,
  threadId,
  findingId,
  title,
  children,
}: {
  reviewId: ReviewId;
  threadId: ThreadId | null;
  findingId: FindingId | null;
  title?: string;
  children: ReactNode;
}) {
  const known = useThreadStore((s) =>
    threadId
      ? !!s.byReview[reviewId]?.threads.some((t) => t.threadId === threadId)
      : false,
  );
  const posted = useAiStore((s) =>
    findingId
      ? s.byReview[reviewId]?.findings.find((x) => x.id === findingId)
      : undefined,
  );
  const open = () => {
    if (threadId && known) focusThread(reviewId, threadId);
    else if (posted) scrollToFinding(reviewId, posted);
  };
  return (
    <button
      type="button"
      className={`ghost ${styles.link}`}
      title={title}
      disabled={!(threadId && known) && !posted}
      onClick={open}
    >
      {children}
    </button>
  );
}

/**
 * "Skipped as already raised (N, k high)": the findings and checklist items
 * set aside, folded unless a high one is among them; nothing when none.
 */
export function SkippedFindings({ reviewId }: { reviewId: ReviewId }) {
  const skipped = useSkippedFindings(reviewId);
  const items = useSkippedChecklist(reviewId);
  const threads = useThreadStore(
    (s) => s.byReview[reviewId]?.threads ?? NO_THREADS,
  );
  // `null`: the user did not toggle it; then it is open with a high one.
  const [toggled, setToggled] = useState<boolean | null>(null);
  const summary = skippedSummary(skipped, items);
  if (summary.total === 0) return null;
  const open = toggled ?? summary.high > 0;
  return (
    <section className={styles.skipped} aria-label="Skipped as already raised">
      <button
        type="button"
        className={`ghost ${styles.dismissedToggle}`}
        aria-expanded={open}
        title="What the review already says: an existing comment or a finding you posted"
        onClick={() => setToggled(!open)}
      >
        <CaretIcon size={12} className={styles.caret} data-open={open} />
        {skippedHeader(summary)}
      </button>
      {open && (
        <>
          <ul className={styles.list}>
            {sortFindings(skipped).map((f) => (
              <SkippedRow
                key={f.id}
                reviewId={reviewId}
                label={`Skipped: ${f.title}`}
                severity={f.severity}
                title={f.title}
                where={findingLocation(f)}
                repeatOf={f.repeatOf}
                target={repeatTarget(f, threads)}
                onShow={() => useAiStore.getState().showAnyway(f)}
              />
            ))}
            {items.map((i) => (
              <SkippedItemRow
                key={i.id}
                reviewId={reviewId}
                item={i}
                target={repeatTarget(i, threads)}
              />
            ))}
          </ul>
          <p className={styles.skippedNote}>
            Show anyway lasts while a rerun of the round finds the same finding
            (same place and title) or item (same text); a rerun that rewords it
            sets it aside again.
          </p>
        </>
      )}
    </section>
  );
}

function SkippedItemRow({
  reviewId,
  item,
  target,
}: {
  reviewId: ReviewId;
  item: ChecklistItem;
  target: string;
}) {
  const showItemAnyway = useAiStore((s) => s.showItemAnyway);
  return (
    <SkippedRow
      reviewId={reviewId}
      label={`Skipped: ${item.text}`}
      severity={null}
      title={item.text}
      where="Look here yourself"
      repeatOf={item.repeatOf}
      target={target}
      onShow={() => showItemAnyway(item)}
    />
  );
}

function SkippedRow({
  reviewId,
  label,
  severity,
  title,
  where,
  repeatOf,
  target,
  onShow,
}: {
  reviewId: ReviewId;
  label: string;
  severity: Finding["severity"] | null;
  title: string;
  where: string;
  repeatOf: RepeatOf | null;
  target: string;
  onShow: () => Promise<void>;
}) {
  return (
    <li
      className={styles.row}
      data-severity={severity ?? undefined}
      data-skipped
      aria-label={label}
    >
      <span className={styles.main}>
        <span className={styles.rowTitle}>{title}</span>
        <span className={`${styles.where} mono`}>{where}</span>
        <span className={styles.tags}>
          {severity && (
            <Badge tone={SEVERITY_TONE[severity]}>
              {SEVERITY_LABEL[severity]}
            </Badge>
          )}
          <PointLink
            reviewId={reviewId}
            threadId={repeatOf?.threadId ?? null}
            findingId={repeatOf?.findingId ?? null}
            title={repeatOf ? REPEAT_REASON_HINT[repeatOf.reason] : undefined}
          >
            repeats {target}
          </PointLink>
          <button
            type="button"
            className={`ghost ${styles.link}`}
            title="List it as a normal one (in Argus only; nothing is posted)"
            onClick={() => void onShow()}
          >
            Show anyway
          </button>
        </span>
      </span>
    </li>
  );
}
