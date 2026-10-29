/**
 * The header's local-clone chip: whether the review's repository has a
 * clone, and where. Three states, read from `cloneStatusStore`:
 * found (the short path; click shows the folder in Finder), configured but
 * missing or no git repository (warning tint; click opens Settings →
 * Repositories on this repository, where clones are set) and not configured
 * (a quiet "No local clone", the same click). The tooltip has the full path, the clone's HEAD and whether
 * it is the review's head. Nothing while the first read is under way.
 */

import type { ReviewRecord } from "../ipc/types";
import { cloneTitle, shortPath } from "../lib/cloneStatus";
import { fileManagerName } from "../state/localFileStore";
import { revealCloneFolder, useCloneStatus } from "../state/cloneStatusStore";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { CloneIcon, CloneMissingIcon } from "./icons";
import styles from "./CloneChip.module.css";

export function CloneChip({ review: r }: { review: ReviewRecord }) {
  // The clone set in Settings, so a change there re-reads the status.
  const repo = `${r.host}/${r.repo}`;
  const setPath = useBackendSettingsStore(
    (s) => s.settings?.clonePaths[repo] ?? null,
  );
  const status = useCloneStatus(r.id, r.headSha, setPath);
  const openSettings = useSettingsRequestStore((s) => s.open);
  if (!status) return null;

  const configured = status.configured && status.path !== null;
  const state = !configured ? "none" : status.found ? "found" : "missing";
  const path = status.path ?? "";
  const label = state === "none" ? "No local clone" : shortPath(path);
  const name =
    state === "none"
      ? "No local clone"
      : state === "missing"
        ? `Local clone not found: ${path}`
        : `Local clone: ${path}`;
  const Icon = state === "missing" ? CloneMissingIcon : CloneIcon;

  return (
    <button
      type="button"
      className={`ghost ${styles.chip}`}
      data-state={state}
      data-slot="clone-chip"
      aria-label={name}
      title={
        state === "found"
          ? cloneTitle(status).replace(
              "Show in folder",
              `Click: show in ${fileManagerName}`,
            )
          : cloneTitle(status)
      }
      onClick={() =>
        state === "found"
          ? void revealCloneFolder(r.id)
          : openSettings("repositories", repo)
      }
    >
      <Icon className={styles.icon} />
      <span className={state === "none" ? undefined : "mono"}>{label}</span>
    </button>
  );
}
