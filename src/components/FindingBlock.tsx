/**
 * One AI finding in the code view: under its line (a CodeMirror block
 * widget, like a thread) or listed at the top of its file card when its
 * line is not in the diff shown. A severity-coloured edge and an "AI" label
 * set it apart from the threads.
 *
 * It reads `aiStore` itself (a widget is its own React root): a "Select for
 * posting" checkbox, Edit (the comment composer's textarea + Preview; Save
 * edit keeps the text, nothing posts), Dismiss / Undismiss, the reason a
 * post failed, and once posted a link to the comment. Posting happens only
 * from the AI island's "Post selected". The keyboard (M6): the focused
 * finding (`aiStore.focused`, kept by `focusStore`) has a ring and the DOM
 * focus; `x` selects it, `e` edits, `d` dismisses.
 */

import { useEffect } from "react";

import type { FindingId, ReviewId, StoredThread } from "../ipc/types";
import {
  SEVERITY_LABEL,
  SEVERITY_TONE,
  findingLocation,
  fromEarlierRound,
  postable,
  relatedLabel,
} from "../lib/findings";
import {
  openPosted,
  useAiStore,
  useCurrentRound,
  useFinding,
  useFindingUi,
} from "../state/aiStore";
import { FOCUS } from "../lib/focusRequests";
import { useThreadStore } from "../state/threadStore";
import { Badge } from "./Badge";
import { CommentComposer } from "./CommentComposer";
import { ExternalLinkIcon } from "./icons";
import { Markdown } from "./Markdown";
import styles from "./FindingBlock.module.css";

const NO_THREADS: StoredThread[] = [];

export function FindingBlock({
  reviewId,
  findingId,
  listed = false,
  onResize,
}: {
  reviewId: ReviewId;
  findingId: FindingId;
  /** Listed at the top of the card: show where it is. */
  listed?: boolean;
  onResize?: () => void;
}) {
  const f = useFinding(reviewId, findingId);
  const ui = useFindingUi(findingId);
  const round = useCurrentRound(reviewId);
  const focused = useAiStore((s) => s.focused === findingId);
  const setSelected = useAiStore((s) => s.setSelected);
  const focus = useAiStore((s) => s.focus);
  const dismiss = useAiStore((s) => s.dismiss);
  const openEditor = useAiStore((s) => s.openEditor);
  const setEditorText = useAiStore((s) => s.setEditorText);
  const saveEditor = useAiStore((s) => s.saveEditor);
  const closeEditor = useAiStore((s) => s.closeEditor);
  const resetEdit = useAiStore((s) => s.resetEdit);
  const posting = useAiStore((s) => !!s.posting[reviewId]);
  const threads = useThreadStore(
    (s) => s.byReview[reviewId]?.threads ?? NO_THREADS,
  );

  const editing = ui.editor !== null;
  // A new height after React commits it (CodeMirror re-measures).
  useEffect(() => {
    const id = requestAnimationFrame(() => onResize?.());
    return () => cancelAnimationFrame(id);
  }, [editing, ui.error, ui.edit, f?.dismissed, f?.postedAt, onResize]);

  if (!f) return null;
  const canPost = postable(f);
  const earlier = fromEarlierRound(f, round);
  const related = relatedLabel(f, threads);

  return (
    <article
      id={`finding-${f.id}`}
      className={styles.finding}
      data-severity={f.severity}
      data-dismissed={f.dismissed}
      data-posted={!!f.postedAt}
      data-failed={!!ui.error}
      data-focused={focused}
      tabIndex={-1}
      aria-label={`AI finding: ${f.title}`}
      aria-describedby={`finding-desc-${f.id}`}
      onMouseDown={() => focus(f.id)}
    >
      <span id={`finding-desc-${f.id}`} className="visually-hidden">
        {`${SEVERITY_LABEL[f.severity]} severity, ${findingLocation(f)}${f.dismissed ? ", dismissed" : ""}${f.postedAt ? ", posted" : ""}`}
      </span>
      <div className={styles.head}>
        {canPost && (
          <input
            type="checkbox"
            className={styles.check}
            checked={ui.selected}
            disabled={posting}
            aria-label={`Select "${f.title}" for posting`}
            title="Select for posting (Post selected in the AI panel)"
            onChange={(e) => setSelected(f.id, e.target.checked)}
          />
        )}
        <span className={styles.ai} aria-hidden>
          AI
        </span>
        <Badge tone={SEVERITY_TONE[f.severity]}>
          {SEVERITY_LABEL[f.severity]}
        </Badge>
        {listed && (
          <span className={`${styles.where} mono`}>{findingLocation(f)}</span>
        )}
        {earlier && <Badge>from round {f.round}</Badge>}
        {related && <Badge>{related}</Badge>}
        {ui.edit !== null && !f.postedAt && <Badge tone="accent">Edited</Badge>}
        {f.dismissed && <Badge>Dismissed</Badge>}
        <span className={styles.spacer} />
        {f.postedAt ? (
          <button
            type="button"
            className="ghost"
            title={f.postedUrl ?? undefined}
            disabled={!f.postedUrl}
            onClick={() => openPosted(f)}
          >
            Posted <ExternalLinkIcon size={11} />
          </button>
        ) : f.dismissed ? (
          <button
            type="button"
            className="ghost"
            onClick={() => void dismiss(f, false)}
          >
            Undismiss
          </button>
        ) : (
          <>
            {!editing && (
              <button
                type="button"
                className="ghost"
                disabled={posting}
                onClick={() => openEditor(f)}
              >
                Edit
              </button>
            )}
            <button
              type="button"
              className="ghost"
              disabled={posting}
              onClick={() => void dismiss(f, true)}
            >
              Dismiss
            </button>
          </>
        )}
      </div>
      <p className={styles.title}>{f.title}</p>
      {ui.error && (
        <p className={styles.error} role="alert">
          Not posted: {ui.error.message}
        </p>
      )}
      {editing ? (
        <div className={styles.editor}>
          <CommentComposer
            label={`Edit finding "${f.title}"`}
            body={ui.editor ?? ""}
            onChange={(text) => setEditorText(f.id, text)}
            onCancel={() => closeEditor(f.id)}
            onResize={onResize}
            focusKey={FOCUS.finding(f.id)}
            primary={{
              label: "Save edit",
              title: posting
                ? "Wait for the post to finish"
                : "Keep this text for posting; nothing is posted yet",
              onClick: () => saveEditor(f),
              disabled: posting,
            }}
            secondary={
              ui.edit !== null
                ? {
                    label: "Reset to AI text",
                    onClick: () => resetEdit(f.id),
                  }
                : undefined
            }
          />
        </div>
      ) : (
        !f.dismissed && (
          <div className={styles.body}>
            <Markdown>{ui.edit ?? f.body}</Markdown>
          </div>
        )
      )}
    </article>
  );
}
