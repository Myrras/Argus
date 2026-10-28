/**
 * Open in editor: the icon button on a file row (`FileTree`, on hover or
 * focus) and in a file card's header (`FileDiffCard`), plus the right-click
 * menu both share (Open in Editor, Open With Default App, Show in Finder —
 * Iris's Local pane menu). Shown only when a local clone is set for the
 * review's repository (`localFileStore`); disabled, with the reason in its
 * tooltip, when the file is not in the working tree. The tooltip also says
 * when the clone is on another commit than the review's head. The menu is
 * `lib/useLocalFileMenu`.
 */

import type { ReviewId } from "../ipc/types";
import { withHint } from "../lib/keymap";
import { openTitle } from "../lib/localFiles";
import { useKeymap } from "../lib/useKeymap";
import { openLocalFile, useLocalFile } from "../state/localFileStore";
import { OpenInEditorIcon } from "./icons";
import styles from "./OpenInEditorButton.module.css";

export function OpenInEditorButton({
  reviewId,
  path,
  className,
}: {
  reviewId: ReviewId;
  path: string;
  className?: string;
}) {
  const file = useLocalFile(reviewId, path);
  const keymap = useKeymap();
  if (!file) return null;
  const title = openTitle(
    file,
    withHint("Open in editor", keymap, "openInEditor"),
  );
  return (
    <button
      type="button"
      className={`ghost ${styles.button} ${className ?? ""}`}
      aria-label={`Open ${path} in editor`}
      title={title}
      disabled={!file.exists}
      data-head-moved={!file.matchesReviewHead && !!file.cloneHead}
      onClick={(e) => {
        // Not a click on the row or header behind it.
        e.stopPropagation();
        void openLocalFile(reviewId, path);
      }}
    >
      <OpenInEditorIcon />
    </button>
  );
}
