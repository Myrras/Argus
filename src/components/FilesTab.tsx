/**
 * The detail island's Files tab: the review's changed files. A placeholder
 * until the diff view lands — the layout is final, so the diff drops in
 * without touching the shell:
 *
 * - `FileTree` slot (`data-slot="file-tree"`): the changed-file tree,
 *   collapsible (`layoutStore.fileTreeOpen`, persisted).
 * - Diff slot (`data-slot="diff"`): the diff of the picked file, with the
 *   threads inline at their lines.
 */

import type { ReviewRecord } from "../ipc/types";
import { reviewRef } from "../lib/reviews";
import { useLayoutStore } from "../state/layoutStore";
import { FileTreeIcon, PanelCloseIcon, PanelOpenIcon } from "./icons";
import styles from "./FilesTab.module.css";

export function FilesTab({ review }: { review: ReviewRecord }) {
  const open = useLayoutStore((s) => s.fileTreeOpen);
  const toggle = useLayoutStore((s) => s.toggleFileTree);

  return (
    <div className={styles.files}>
      <aside
        className={styles.tree}
        data-open={open ? "true" : "false"}
        data-slot="file-tree"
        aria-label="Changed files"
      >
        <div className={styles.treeHeader}>
          {open && (
            <>
              <FileTreeIcon className={styles.treeIcon} />
              <span className={styles.treeTitle}>Files</span>
            </>
          )}
          <button
            type="button"
            className={`ghost ${styles.treeToggle}`}
            aria-expanded={open}
            aria-label={
              open ? "Collapse the file tree" : "Expand the file tree"
            }
            title={open ? "Collapse the file tree" : "Expand the file tree"}
            onClick={toggle}
          >
            {open ? <PanelCloseIcon /> : <PanelOpenIcon />}
          </button>
        </div>
        {open && (
          <p className={styles.placeholder}>The changed files go here.</p>
        )}
      </aside>
      <div
        className={styles.diff}
        data-slot="diff"
        aria-label={`Changes in ${reviewRef(review)}`}
      >
        <p className={styles.placeholder}>
          Files changed: coming soon. The diff with your threads at their lines
          shows here.
        </p>
      </div>
    </div>
  );
}
