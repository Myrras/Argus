/**
 * The header's local-clone chip: how a clone's folder reads short, and what
 * the tooltip says (`CloneStatus` from the core's `clone_status`).
 */

import type { CloneStatus } from "../ipc/types";

/** The user's home folder at the start of a path: macOS, Linux, Windows. */
const HOME =
  /^(?:\/Users\/[^/]+|\/home\/[^/]+|[A-Za-z]:\/Users\/[^/]+)(?=\/|$)/;

/**
 * A folder as a short label: the home folder as `~`, and anything deeper
 * than two parts cut to its last one, `~/…/elements`. Short paths stay whole.
 */
export function shortPath(path: string): string {
  const slashed = path.replace(/\\/g, "/").replace(/\/+$/, "");
  const home = HOME.exec(slashed);
  const head = home ? "~" : slashed.startsWith("/") ? "" : null;
  const rest = (home ? slashed.slice(home[0].length) : slashed).split("/");
  const parts = rest.filter(Boolean);
  if (parts.length <= (home ? 2 : 1) && head !== null) {
    return `${head}/${parts.join("/")}`.replace(/\/$/, "") || "/";
  }
  const last = parts[parts.length - 1] ?? "";
  if (head === null) {
    // A relative or drive-less path: keep the first part for context.
    return parts.length <= 2 ? parts.join("/") : `${parts[0]}/…/${last}`;
  }
  return `${head}/…/${last}`;
}

/** What the clone's head is, against the review's. */
export function headLine(status: CloneStatus): string {
  if (!status.head) return "HEAD unknown";
  const sha = status.head.slice(0, 7);
  return status.matchesReviewHead
    ? `HEAD ${sha}: at the review's head`
    : `HEAD ${sha}: on another commit (Argus reads objects only)`;
}

/** The chip's tooltip, one fact per line. */
export function cloneTitle(status: CloneStatus): string {
  if (!status.configured || !status.path) {
    return "No local clone is set for this repository\nClick: set one in Settings → Repositories";
  }
  if (!status.found) {
    return `Local clone not found: ${status.path}\nClick: check it in Settings → Repositories`;
  }
  return `${status.path}\n${headLine(status)}\nShow in folder`;
}
