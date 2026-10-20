/**
 * The review view's navigation (`data-slot="file-tree"` body): a
 * Conversation entry with the user's open threads ("2 open") first, then the changed files as folders with single-child chains
 * compacted, or a flat path list. Per file
 * its status, +/− counts, the user's open thread count and
 * a Viewed checkbox. Selecting an entry scrolls the page to it.
 *
 * Open in editor: `fileAction` renders a per-file button (shown on hover or
 * focus, `OpenInEditorButton`) and `onFileContextMenu` the file menu.
 */

import { useState } from "react";

import type { FileEntry } from "../ipc/types";
import { buildTree, type TreeNode } from "../lib/fileTree";
import type { FileListMode } from "../state/layoutStore";
import {
  FileStatusMark,
  LineStats,
  MyOpenThreads,
  OpenCount,
} from "./FileStatusMark";
import { CaretIcon, ThreadsIcon } from "./icons";
import styles from "./FileTree.module.css";

export interface FileTreeProps {
  files: readonly FileEntry[];
  mode: FileListMode;
  selected: string | null;
  /** Per path: the user's open threads (`myOpenCount`). */
  counts: ReadonlyMap<string, number>;
  onSelect: (path: string) => void;
  onToggleViewed: (file: FileEntry) => void;
  /** Shown when `files` is empty; `null` shows nothing. */
  empty?: string | null;
  /** The Conversation entry above the files. */
  conversation?: {
    /** The user's open threads ("2 open"). */
    open: number;
    /** All its threads; only in the tooltip. */
    total: number;
    selected: boolean;
    onSelect: () => void;
  };
  /** A per-file action before the Viewed box (Open in editor). */
  fileAction?: (file: FileEntry) => React.ReactNode;
  /** Right-click on a file row (the Open in editor menu). */
  onFileContextMenu?: (file: FileEntry, e: React.MouseEvent) => void;
}

export function FileTree(props: FileTreeProps) {
  const { files, mode } = props;
  const [closed, setClosed] = useState<ReadonlySet<string>>(new Set());
  const toggleDir = (path: string) =>
    setClosed((c) => {
      const next = new Set(c);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  const c = props.conversation;
  const rows =
    mode === "tree"
      ? renderNodes(buildTree(files), 0, closed, toggleDir, props)
      : files.map((f) => (
          <FileRow key={f.path} file={f} depth={0} label={f.path} {...props} />
        ));
  return (
    <>
      <ul className={styles.tree} role="tree" aria-label="Changed files">
        {c && (
          <li
            role="treeitem"
            aria-selected={c.selected}
            className={`${styles.fileItem} ${styles.conversationItem}`}
            data-selected={c.selected}
          >
            <button
              type="button"
              className={styles.file}
              style={{ paddingLeft: indent(0) }}
              onClick={c.onSelect}
              title={`Comments on the review as a whole (${conversationLabel(c.open, c.total)})`}
            >
              <ThreadsIcon className={styles.conversationIcon} />
              <span className={styles.name}>Conversation</span>
              <OpenCount
                count={c.open}
                label={conversationLabel(c.open, c.total)}
              />
            </button>
          </li>
        )}
        {rows}
      </ul>
      {files.length === 0 && props.empty !== null && (
        <p className={styles.empty}>{props.empty ?? "No files match."}</p>
      )}
    </>
  );
}

function renderNodes(
  nodes: readonly TreeNode[],
  depth: number,
  closed: ReadonlySet<string>,
  toggleDir: (path: string) => void,
  props: FileTreeProps,
): React.ReactNode[] {
  return nodes.flatMap((n) => {
    if (n.kind === "file") {
      return [
        <FileRow
          key={n.file.path}
          file={n.file}
          depth={depth}
          label={n.name}
          {...props}
        />,
      ];
    }
    const open = !closed.has(n.path);
    return [
      <li
        key={`dir:${n.path}`}
        role="treeitem"
        aria-expanded={open}
        className={styles.dirItem}
      >
        <button
          type="button"
          className={styles.dir}
          style={{ paddingLeft: indent(depth) }}
          onClick={() => toggleDir(n.path)}
          title={n.path}
        >
          <CaretIcon size={12} className={styles.caret} data-open={open} />
          <span className={styles.name}>{n.name}</span>
        </button>
      </li>,
      ...(open
        ? renderNodes(n.children, depth + 1, closed, toggleDir, props)
        : []),
    ];
  });
}

function conversationLabel(open: number, total: number): string {
  const n = (k: number) => `${k} ${k === 1 ? "thread" : "threads"}`;
  return `${open} open ${open === 1 ? "thread" : "threads"} of yours, ${n(total)} in all`;
}

/** Tokens are px steps of --sp-3; inline because depth is data. */
function indent(depth: number): string {
  return `calc(var(--sp-2) + ${depth} * var(--sp-3))`;
}

function FileRow({
  file: f,
  depth,
  label,
  selected,
  counts,
  onSelect,
  onToggleViewed,
  fileAction,
  onFileContextMenu,
}: FileTreeProps & { file: FileEntry; depth: number; label: string }) {
  const count = counts.get(f.path) ?? 0;
  const isSelected = selected === f.path;
  return (
    <li
      role="treeitem"
      aria-selected={isSelected}
      className={styles.fileItem}
      data-selected={isSelected}
      data-viewed={f.viewed}
      onContextMenu={
        onFileContextMenu ? (e) => onFileContextMenu(f, e) : undefined
      }
    >
      <button
        type="button"
        className={styles.file}
        style={{ paddingLeft: indent(depth) }}
        onClick={() => onSelect(f.path)}
        title={f.oldPath ? `${f.oldPath} → ${f.path}` : f.path}
      >
        <FileStatusMark status={f.status} />
        <span className={styles.name}>{label}</span>
        <MyOpenThreads count={count} />
        <LineStats additions={f.additions} deletions={f.deletions} />
      </button>
      {fileAction && <span className={styles.fileAction}>{fileAction(f)}</span>}
      <input
        type="checkbox"
        className={styles.viewed}
        checked={f.viewed}
        onChange={() => onToggleViewed(f)}
        aria-label={`Viewed ${f.path}`}
        title="Viewed"
      />
    </li>
  );
}
