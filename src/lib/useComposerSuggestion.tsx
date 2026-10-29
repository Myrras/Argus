/**
 * What a composer needs to write a suggestion: whether it can (and why
 * not), the lines to prefill at the review head, the file's indentation,
 * and the preview of a suggestion block as the `SuggestedChange` diff a
 * posted one shows. The rules are `lib/suggestions` (`*SuggestionPlan`);
 * this loads the file (`codeViewStore.loadContents`, cached) and wires
 * them to `CommentComposer`.
 *
 * Reading only: nothing here writes, and nothing posts.
 */

import { useEffect, useState } from "react";

import type { ProviderId, ReviewId } from "../ipc/types";
import { SuggestedChange } from "../components/SuggestedChange";
import type { RenderFence } from "../components/Markdown";
import { loadContents } from "../state/codeViewStore";
import {
  indentUnit,
  linesFromContents,
  parseSuggestionInfo,
  planAnchor,
  type LineRange,
  type SuggestionPlan,
} from "./suggestions";

export interface ComposerSuggestion {
  provider: ProviderId;
  /** The comment's lines at the head; null when it takes no suggestion. */
  range: LineRange | null;
  /** Why "Insert suggestion" is off right now; null when it is on. */
  blocked: string | null;
  /** The lines to prefill (null until loaded, or when blocked). */
  lines: string[] | null;
  /** The file's indentation step, for Tab in a suggestion. */
  indent: string;
  /** Previews a suggestion block as its diff (null: other code). */
  renderFence?: RenderFence;
  /** The range the last inserted suggestion was written for. */
  insertedFor: LineRange | null;
  onInserted?: (range: LineRange) => void;
  /** Put the comment back on `range` (a new comment only). */
  onUseRange?: (range: LineRange) => void;
}

type Loaded =
  { key: string; state: "ok"; text: string } | { key: string; state: "error" };

export function useComposerSuggestion({
  reviewId,
  provider,
  plan,
  insertedFor = null,
  onInserted,
  onUseRange,
  onResize,
}: {
  reviewId: ReviewId;
  /** Null when the review is not known (no suggestion tools then). */
  provider: ProviderId | null;
  plan: SuggestionPlan;
  insertedFor?: LineRange | null;
  onInserted?: (range: LineRange) => void;
  onUseRange?: (range: LineRange) => void;
  onResize?: () => void;
}): ComposerSuggestion | undefined {
  const sha = plan.ok ? plan.sha : null;
  const path = plan.ok ? plan.path : null;
  const key = sha && path ? `${reviewId}|${sha}|${path}` : null;
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  useEffect(() => {
    if (!key || !sha || !path) return;
    let live = true;
    loadContents(reviewId, sha, path).then(
      (text) => live && setLoaded({ key, state: "ok", text }),
      () => live && setLoaded({ key, state: "error" }),
    );
    return () => {
      live = false;
    };
  }, [key, reviewId, sha, path]);

  if (!provider) return undefined;
  if (!plan.ok) {
    return {
      provider,
      range: null,
      blocked: plan.reason,
      lines: null,
      indent: "    ",
      insertedFor: null,
    };
  }
  const file = loaded?.key === key ? loaded : null;
  const lines =
    file?.state === "ok" ? linesFromContents(file.text, plan.range) : null;
  const blocked = !file
    ? "Loading the file…"
    : file.state === "error"
      ? "Couldn't read the file at the head, so there are no lines to suggest on"
      : !lines
        ? "These lines are not in the file at the head"
        : null;
  const range = plan.range;
  const source = { sha: plan.sha, path: plan.path };
  const renderFence: RenderFence = (info, code) => {
    const spec = parseSuggestionInfo(info);
    // GitHub reads no offsets: such a fence is plain code there.
    if (!spec || (provider === "github" && spec.kind === "relative")) {
      return null;
    }
    return (
      <SuggestedChange
        spec={spec}
        body={code}
        reviewId={reviewId}
        anchor={planAnchor(range)}
        source={source}
        provider={provider}
        onResize={onResize}
      />
    );
  };
  return {
    provider,
    range,
    blocked,
    lines,
    indent: file?.state === "ok" ? indentUnit(file.text) : "    ",
    renderFence,
    insertedFor,
    onInserted,
    onUseRange,
  };
}
