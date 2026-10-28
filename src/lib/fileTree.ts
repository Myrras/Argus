/**
 * The review view's file list as a directory tree (folders first, single-child
 * folder chains compacted as `src/lib`) or a flat path list, and the filter.
 * Pure. The diff column stacks the files in the same order, so j/k, the tree
 * and the scroll position agree.
 */

import type { FileEntry } from "../ipc/types";

export interface TreeDir {
  kind: "dir";
  /** Shown name (a compacted chain reads `a/b`). */
  name: string;
  /** Full folder path, the key for its open/closed state. */
  path: string;
  children: TreeNode[];
}

export interface TreeFile {
  kind: "file";
  name: string;
  file: FileEntry;
}

export type TreeNode = TreeDir | TreeFile;

const byName = (x: { name: string }, y: { name: string }) =>
  x.name.localeCompare(y.name, undefined, { sensitivity: "base" });

export function buildTree(files: readonly FileEntry[]): TreeNode[] {
  const root: TreeDir = { kind: "dir", name: "", path: "", children: [] };
  for (const file of files) {
    const parts = file.path.split("/");
    const name = parts.pop() ?? file.path;
    let dir = root;
    for (const part of parts) {
      const path = dir.path ? `${dir.path}/${part}` : part;
      let next = dir.children.find(
        (c): c is TreeDir => c.kind === "dir" && c.path === path,
      );
      if (!next) {
        next = { kind: "dir", name: part, path, children: [] };
        dir.children.push(next);
      }
      dir = next;
    }
    dir.children.push({ kind: "file", name, file });
  }
  return sortAndCompact(root.children);
}

function sortAndCompact(nodes: TreeNode[]): TreeNode[] {
  const dirs = nodes
    .filter((n): n is TreeDir => n.kind === "dir")
    .map((d) => {
      let dir = { ...d, children: sortAndCompact(d.children) };
      // A folder holding just one folder reads as one row.
      while (dir.children.length === 1 && dir.children[0].kind === "dir") {
        const only = dir.children[0];
        dir = {
          ...only,
          name: `${dir.name}/${only.name}`,
        };
      }
      return dir;
    })
    .sort(byName);
  const files = nodes
    .filter((n): n is TreeFile => n.kind === "file")
    .sort(byName);
  return [...dirs, ...files];
}

/** The files in tree order (what the tree shows top to bottom). */
export function treeOrder(nodes: readonly TreeNode[]): FileEntry[] {
  const out: FileEntry[] = [];
  const walk = (list: readonly TreeNode[]) => {
    for (const n of list) {
      if (n.kind === "file") out.push(n.file);
      else walk(n.children);
    }
  };
  walk(nodes);
  return out;
}

/** The files in display order for the chosen mode. */
export function orderFiles(
  files: readonly FileEntry[],
  mode: "tree" | "flat",
): FileEntry[] {
  if (mode === "tree") return treeOrder(buildTree(files));
  return [...files].sort((x, y) =>
    x.path.localeCompare(y.path, undefined, { sensitivity: "base" }),
  );
}

/** Case-insensitive match on the path (or the path before a rename). */
export function filterFiles(
  files: readonly FileEntry[],
  query: string,
): FileEntry[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...files];
  return files.filter(
    (f) =>
      f.path.toLowerCase().includes(q) ||
      (f.oldPath?.toLowerCase().includes(q) ?? false),
  );
}
