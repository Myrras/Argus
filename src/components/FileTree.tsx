/**
 * The Files tab's file list (`data-slot="file-tree"` body): folders with
 * single-child chains compacted, or a flat path list. Per file its status,
 * +/− counts, thread count (accent when some are the user's) and a Viewed
 * checkbox. Selecting a file scrolls the diff column to it.
 */

import { useState } from "react";

import type { FileEntry } from "../ipc/types";
import { buildTree, type TreeNode } from "../lib/fileTree";
import type { FileListMode } from "../state/layoutStore";
import { FileStatusMark, LineStats, ThreadCount } from "./FileStatusMark";
import { CaretIcon } from "./icons";
import styles from "./FileTree.module.css";

export interface FileTreeProps {
  files: readonly FileEntry[];
  mode: FileListMode;
  selected: string | null;
  counts: ReadonlyMap<string, { total: number; mine: number }>;
  onSelect: (path: string) => void;
  onToggleViewed: (file: FileEntry) => void;
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

  if (files.length === 0) {
    return <p className={styles.empty}>No files match.</p>;
  }
  const rows =
    mode === "tree"
      ? renderNodes(buildTree(files), 0, closed, toggleDir, props)
      : files.map((f) => (
          <FileRow key={f.path} file={f} depth={0} label={f.path} {...props} />
        ));
  return (
    <ul className={styles.tree} role="tree" aria-label="Changed files">
      {rows}
    </ul>
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
}: FileTreeProps & { file: FileEntry; depth: number; label: string }) {
  const count = counts.get(f.path) ?? { total: 0, mine: 0 };
  const isSelected = selected === f.path;
  return (
    <li
      role="treeitem"
      aria-selected={isSelected}
      className={styles.fileItem}
      data-selected={isSelected}
      data-viewed={f.viewed}
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
        <ThreadCount {...count} />
        <LineStats additions={f.additions} deletions={f.deletions} />
      </button>
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
