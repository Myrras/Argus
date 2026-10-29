/**
 * Settings → Repositories: every repository Argus knows with its setup
 * (`list_repositories`: the stored reviews' repositories plus the ones the
 * settings name, sorted by the core). Read-only: the setup is saved through
 * `backendSettingsStore.save`. Re-read while the tab shows: when it opens,
 * after every settings change and after every refresh (a repository seen
 * for the first time appears). A failed read keeps the last list and says
 * why in `error`.
 */

import { create } from "zustand";

import { listRepositories, revealRepoClone, toIpcError } from "../ipc/commands";
import { onReviewsRefreshed } from "../ipc/events";
import type { RepoSetup } from "../ipc/types";
import { latestOnly } from "../lib/latest";
import { fileManagerName } from "./localFileStore";
import { toastError } from "./toastStore";

interface RepositoriesState {
  /** `null` until the first read answered. */
  repos: RepoSetup[] | null;
  error: string | null;
  load: () => Promise<void>;
}

const reads = latestOnly<"repos">();

export const useRepositoriesStore = create<RepositoriesState>((set) => ({
  repos: null,
  error: null,
  load: async () => {
    const current = reads.begin("repos");
    try {
      const repos = await listRepositories();
      if (!current()) return;
      set({ repos: Array.isArray(repos) ? repos : [], error: null });
    } catch (e) {
      if (!current()) return;
      set({ error: toIpcError(e).message });
    }
  },
}));

/** Re-read after every refresh; resolves to an unsubscribe. */
export function subscribeRepositoriesEvents(): Promise<() => void> {
  return onReviewsRefreshed(() => void useRepositoriesStore.getState().load());
}

/** Show the repository's clone folder in the file manager. */
export async function revealRepositoryClone(repo: string): Promise<boolean> {
  try {
    await revealRepoClone(repo);
    return true;
  } catch (e) {
    toastError(
      `Could not show the clone in ${fileManagerName}`,
      toIpcError(e).message,
    );
    return false;
  }
}
