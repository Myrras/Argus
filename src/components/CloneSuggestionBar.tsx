/**
 * The offer under the review header when the repository has no local clone
 * set and Argus found one on disk: "Found a local clone of <repo> at
 * <path>. Use it?".
 *
 * "Use it" stores it as the repository's clone, "No thanks" is remembered
 * for good (neither comes back), "Set up…" opens Settings → Repositories on
 * the repository (another folder, its review skill and language), and the
 * × only hides the offer for this session. Nothing shows while a clone is set, while the first scan runs,
 * or when nothing was found. See `cloneSuggestStore`.
 */

import { useEffect } from "react";

import type { ReviewRecord } from "../ipc/types";
import { shortPath } from "../lib/cloneStatus";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { useCloneSuggestStore } from "../state/cloneSuggestStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { CloneIcon, CloseIcon } from "./icons";
import styles from "./CloneSuggestionBar.module.css";

export function CloneSuggestionBar({ review: r }: { review: ReviewRecord }) {
  const repo = `${r.host}/${r.repo}`;
  // The clone set in Settings: nothing is offered while there is one, and a
  // change there asks again.
  const setPath = useBackendSettingsStore(
    (s) => s.settings?.clonePaths[repo] ?? null,
  );
  const suggestion = useCloneSuggestStore((s) => s.byReview[r.id]);
  const dismissed = useCloneSuggestStore((s) => !!s.dismissed[repo]);
  const busy = useCloneSuggestStore((s) => !!s.busy[r.id]);
  const load = useCloneSuggestStore((s) => s.load);
  const use = useCloneSuggestStore((s) => s.use);
  const ignore = useCloneSuggestStore((s) => s.ignore);
  const dismiss = useCloneSuggestStore((s) => s.dismiss);
  const openSettings = useSettingsRequestStore((s) => s.open);

  useEffect(() => {
    if (setPath || dismissed) return;
    void load(r.id);
  }, [r.id, setPath, dismissed, load]);

  if (setPath || dismissed || !suggestion) return null;

  return (
    <div className={styles.bar} data-slot="clone-suggestion" role="status">
      <CloneIcon className={styles.icon} />
      <span className={styles.text}>
        Found a local clone of <strong>{suggestion.repo}</strong> at{" "}
        <span className="mono" title={suggestion.path}>
          {shortPath(suggestion.path)}
        </span>
        . Use it?
      </span>
      <span className={styles.actions}>
        <button
          type="button"
          className="primary"
          disabled={busy}
          title={`Use ${suggestion.path} as the local clone of ${suggestion.repo}`}
          onClick={() => void use(r.id, suggestion.path)}
        >
          Use it
        </button>
        <button
          type="button"
          disabled={busy}
          title="Never offer a clone for this repository again"
          onClick={() => void ignore(r.id)}
        >
          No thanks
        </button>
        <button
          type="button"
          className="ghost"
          title={`Set up ${repo} in Settings → Repositories: another folder, its review skill and language`}
          onClick={() => openSettings("repositories", repo)}
        >
          Set up…
        </button>
        <button
          type="button"
          className={`iconButton ${styles.close}`}
          aria-label="Hide this suggestion for now"
          title="Hide this suggestion for now"
          onClick={() => dismiss(repo)}
        >
          <CloseIcon size={13} />
        </button>
      </span>
    </div>
  );
}
