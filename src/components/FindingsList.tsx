/**
 * The AI island's findings: grouped by severity (high → medium → low), then
 * path and line. Each row has its "select for posting" checkbox, the title
 * (a click scrolls the code view to the finding), where it is, "from round
 * N" for an earlier round's unposted one, "Edited", a link once posted, and
 * the reason when its last post failed. Dismissed findings stay hidden
 * behind the "Show dismissed" toggle.
 *
 * The header's "Post selected (N)" is the only way findings reach the
 * platform: each posted one gets its link, a failed one stays selected with
 * its reason, and the rest are posted regardless.
 */

import type { Finding, ReviewId } from "../ipc/types";
import {
  SEVERITY_LABEL,
  SEVERITY_TONE,
  findingLocation,
  fromEarlierRound,
  groupBySeverity,
  postable,
} from "../lib/findings";
import {
  openPosted,
  useAiStore,
  useCurrentRound,
  useFindings,
} from "../state/aiStore";
import { useCodeViewStore } from "../state/codeViewStore";
import { Badge } from "./Badge";
import { ExternalLinkIcon } from "./icons";
import styles from "./FindingsList.module.css";

/** Select the finding's file and scroll to the finding once it rendered. */
function scrollToFinding(reviewId: ReviewId, f: Finding) {
  useAiStore.getState().revealFinding(f);
  useCodeViewStore.getState().select(reviewId, f.path);
  let tries = 0;
  const tick = () => {
    const el = document.getElementById(`finding-${f.id}`);
    if (el) {
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
  const canPost = postable(f);
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
