/**
 * A thread's AI verdict in its header: addressed / partly addressed /
 * untouched, the AI's reason in the tooltip, and a menu to set your own
 * call or clear it (`set_verdict_override`). An override shows as such
 * ("you: addressed · AI: partly"). An Argus-only hint: it never changes the
 * thread on the platform, and the tooltip says so. Renders nothing without a
 * verdict or with the AI pass off.
 */

import { useState } from "react";

import type {
  ProviderId,
  ReviewId,
  ThreadId,
  VerdictLabel,
} from "../ipc/types";
import { VERDICT_LABEL, VERDICT_SHORT } from "../lib/findings";
import { PROVIDER_LABEL } from "../lib/reviews";
import { useAiStore, useVerdict } from "../state/aiStore";
import { ContextMenu, type ContextMenuItem } from "./ContextMenu";
import styles from "./VerdictBadge.module.css";

const LABELS: VerdictLabel[] = ["addressed", "partly", "untouched"];

export function VerdictBadge({
  reviewId,
  threadId,
  provider,
}: {
  reviewId: ReviewId;
  threadId: ThreadId;
  provider: ProviderId;
}) {
  const v = useVerdict(reviewId, threadId);
  const setOverride = useAiStore((s) => s.setOverride);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  if (!v || !v.label) return null;

  const override = v.userOverride;
  const text = override
    ? `you: ${VERDICT_SHORT[override]}${v.aiLabel ? ` · AI: ${VERDICT_SHORT[v.aiLabel]}` : ""}`
    : `AI: ${VERDICT_LABEL[v.label].toLowerCase()}`;
  const reason = v.aiReason?.trim() || "The AI gave no reason.";
  const title = [
    v.aiLabel ? `AI: ${VERDICT_LABEL[v.aiLabel]} — ${reason}` : reason,
    override ? `Your call: ${VERDICT_LABEL[override]}.` : null,
    `A hint inside Argus only: it never changes the thread on ${PROVIDER_LABEL[provider]}. Click to set your own.`,
  ]
    .filter(Boolean)
    .join("\n\n");

  const items: ContextMenuItem[] = [
    ...LABELS.map((label) => ({
      label: `Mark ${VERDICT_LABEL[label].toLowerCase()}`,
      disabled: override === label,
      onSelect: () => void setOverride(reviewId, threadId, label),
    })),
    {
      label: "Clear my override",
      disabled: !override,
      onSelect: () => void setOverride(reviewId, threadId, null),
    },
  ];

  return (
    <>
      <button
        type="button"
        className={styles.badge}
        data-label={v.label}
        data-override={!!override}
        aria-label={`Verdict: ${text}`}
        aria-haspopup="menu"
        title={title}
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setMenu({ x: r.left, y: r.bottom + 4 });
        }}
      >
        {text}
      </button>
      <ContextMenu
        open={menu !== null}
        x={menu?.x ?? 0}
        y={menu?.y ?? 0}
        items={items}
        onClose={() => setMenu(null)}
      />
    </>
  );
}
