/**
 * Open in editor: the right-click file menu of the file tree and the file
 * cards (Open in Editor, Open With Default App, Show in Finder — Iris's
 * Local pane menu).
 */

import { useCallback, useState, type MouseEvent } from "react";

import type { ReviewId } from "../ipc/types";
import { ContextMenu } from "../components/ContextMenu";
import { shortcutHint } from "./keymap";
import { localFileMenuItems } from "./localFiles";
import { useKeymap } from "./useKeymap";

/**
 * The file menu on right-click; `onContextMenu(path, e)` opens it, `menu`
 * renders it. Nothing opens without a clone (the native menu stays off too).
 */
export function useLocalFileMenu(reviewId: ReviewId) {
  const keymap = useKeymap();
  const [at, setAt] = useState<{ path: string; x: number; y: number } | null>(
    null,
  );
  const onContextMenu = useCallback(
    (path: string, e: MouseEvent) => {
      if (localFileMenuItems(reviewId, path).length === 0) return;
      e.preventDefault();
      setAt({ path, x: e.clientX, y: e.clientY });
    },
    [reviewId],
  );
  const close = useCallback(() => setAt(null), []);
  const menu = (
    <ContextMenu
      open={at !== null}
      x={at?.x ?? 0}
      y={at?.y ?? 0}
      items={
        at
          ? localFileMenuItems(reviewId, at.path, {
              open: shortcutHint(keymap, "openInEditor"),
              reveal: shortcutHint(keymap, "showInFinder"),
            })
          : []
      }
      onClose={close}
    />
  );
  return { onContextMenu, menu };
}
