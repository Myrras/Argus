/**
 * Settings → Repositories: the setup of every repository Argus knows, one
 * card each (`repositoriesStore`, the core's `list_repositories`: the stored
 * reviews' repositories plus the ones the settings name, those with open
 * reviews first). A repository seen for the first time shows up after the
 * refresh that found it, marked "Not set up"; nothing else nags.
 *
 * Per card, all keyed `host/repo` in the settings:
 * - Local clone (`clonePaths`): the folder, whether it is there and a git
 *   repository, Choose… (the dialog plugin's folder picker), Show in
 *   Finder, Clear.
 * - Review skill (`reviewSkills`): `ReviewSkillField`.
 * - Review language (`reviewLanguages`): `ReviewLanguageField`, with the AI
 *   tab's default as "Default (…)".
 *
 * Choices apply at once; typed text (another language's name, a repository
 * added by hand) has its own Save. Failures are toasts (`backendSettingsStore`).
 * Opened on a repository (`settingsRequestStore.open("repositories", key)`,
 * from the clone chip or the clone suggestion), the tab clears the filter,
 * scrolls the card into view and highlights it.
 */

import { useEffect, useRef, useState } from "react";

import type { CloneState, RepoSetup } from "../ipc/types";
import { toIpcError } from "../ipc/commands";
import {
  cloneError,
  normalizeRepo,
  withClone,
  withoutClone,
} from "../lib/clonePaths";
import { shortPath } from "../lib/cloneStatus";
import {
  CLONE_STATE_LABEL,
  FILTER_FROM,
  filterRepos,
  globalLanguage,
  isSetUp,
  languageFor,
  withLanguage,
} from "../lib/repositories";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { fileManagerName } from "../state/localFileStore";
import {
  revealRepositoryClone,
  subscribeRepositoriesEvents,
  useRepositoriesStore,
} from "../state/repositoriesStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { toastError } from "../state/toastStore";
import { PlatformBadge } from "./Badge";
import { CloneIcon, CloneMissingIcon } from "./icons";
import { ReviewLanguageField } from "./ReviewLanguageField";
import { ReviewSkillField } from "./ReviewSkillPicker";
import own from "./RepositoriesTab.module.css";
import styles from "./SettingsModal.module.css";

/** Native folder picker for a local clone. */
async function pickFolder(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({
    multiple: false,
    directory: true,
    title: "Choose the local clone",
  });
  return typeof picked === "string" ? picked : null;
}

export function RepositoriesTab() {
  const settings = useBackendSettingsStore((s) => s.settings);
  const repos = useRepositoriesStore((s) => s.repos);
  const error = useRepositoriesStore((s) => s.error);
  const load = useRepositoriesStore((s) => s.load);
  const requested = useSettingsRequestStore((s) => s.repo);
  const seq = useSettingsRequestStore((s) => s.seq);
  const [query, setQuery] = useState("");
  const [highlighted, setHighlighted] = useState<string | null>(null);
  const cards = useRef(new Map<string, HTMLElement>());

  // On open and after every settings change (here, the chip's "Use it", a
  // save elsewhere): the clone states and the list follow.
  useEffect(() => {
    void load();
  }, [settings, load]);
  // After every refresh while the tab shows: new repositories appear.
  useEffect(() => {
    let off: (() => void) | null = null;
    let gone = false;
    subscribeRepositoriesEvents()
      .then((unsubscribe) => {
        if (gone) unsubscribe();
        else off = unsubscribe;
      })
      .catch(() => undefined);
    return () => {
      gone = true;
      off?.();
    };
  }, []);
  // Asked for a repository: show it, whatever the filter said.
  useEffect(() => {
    if (!requested) return;
    setQuery("");
    setHighlighted(requested);
  }, [requested, seq]);
  useEffect(() => {
    if (!highlighted || !repos) return;
    cards.current
      .get(highlighted)
      ?.scrollIntoView?.({ block: "nearest", behavior: "smooth" });
  }, [highlighted, repos, seq]);

  const all = repos ?? [];
  const shown = filterRepos(all, query);

  return (
    <section className={styles.section}>
      <h3 className={styles.sectionLabel}>Repositories</h3>
      <small className={styles.rowHelp}>
        Each repository Argus has seen a review of, with its local clone, the
        review skill and the language the AI analysis writes in. New ones are
        added as reviews arrive.
      </small>
      {all.length >= FILTER_FROM && (
        <input
          type="search"
          value={query}
          placeholder="Filter repositories"
          aria-label="Filter repositories"
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          onChange={(e) => setQuery(e.target.value)}
        />
      )}
      {error && (
        <p className={styles.error} role="alert">
          Couldn&apos;t list the repositories: {error}
        </p>
      )}
      {repos !== null && all.length === 0 && (
        <p className={styles.empty}>
          No repositories yet. They show here once Argus lists a review, or add
          one below.
        </p>
      )}
      {all.length > 0 && shown.length === 0 && (
        <p className={styles.empty}>
          No repository matches &ldquo;{query.trim()}&rdquo;.
        </p>
      )}
      {shown.map((repo, i) => (
        <RepoCard
          key={repo.key}
          index={i}
          repo={repo}
          highlighted={highlighted === repo.key}
          cardRef={(el) => {
            if (el) cards.current.set(repo.key, el);
            else cards.current.delete(repo.key);
          }}
        />
      ))}
      <AddRepository />
    </section>
  );
}

function RepoCard({
  repo,
  index,
  highlighted,
  cardRef,
}: {
  repo: RepoSetup;
  index: number;
  highlighted: boolean;
  cardRef: (el: HTMLElement | null) => void;
}) {
  const settings = useBackendSettingsStore((s) => s.settings);
  const save = useBackendSettingsStore((s) => s.save);
  const key = repo.key;
  return (
    <div
      ref={cardRef}
      className={`${styles.card} ${own.card}`}
      role="group"
      aria-label={key}
      data-slot="repo-card"
      data-highlighted={highlighted ? "true" : undefined}
    >
      <div className={styles.cardHeader}>
        {repo.provider && <PlatformBadge provider={repo.provider} />}
        <span className={`${styles.cardName} ${own.name}`} title={key}>
          {key}
        </span>
        <span className={`${own.meta} ${styles.pushRight}`}>
          {repo.openReviews > 0 && (
            <span className={own.count}>{repo.openReviews} open</span>
          )}
          {!isSetUp(settings, key) && (
            <span className={own.tag}>Not set up</span>
          )}
        </span>
      </div>
      <div className={own.setting}>
        <span className={own.label}>Local clone</span>
        <CloneField repo={repo} />
      </div>
      <div className={own.setting}>
        <span className={own.label}>Review skill</span>
        <ReviewSkillField repo={key} />
      </div>
      <div className={own.setting}>
        <label className={own.label} htmlFor={`repo-language-${index}`}>
          Review language
        </label>
        <ReviewLanguageField
          id={`repo-language-${index}`}
          label={`Review language of ${key}`}
          value={languageFor(settings, key)}
          fallback={globalLanguage(settings)}
          disabled={!settings}
          onChange={(language) =>
            settings &&
            void save({
              reviewLanguages: withLanguage(
                settings.reviewLanguages,
                key,
                language,
              ),
            })
          }
        />
      </div>
    </div>
  );
}

/** The folder, what it is on disk, and Choose… / Show / Clear. */
function CloneField({ repo }: { repo: RepoSetup }) {
  const settings = useBackendSettingsStore((s) => s.settings);
  const save = useBackendSettingsStore((s) => s.save);
  const key = repo.key;
  const clones = settings?.clonePaths ?? {};
  const path = clones[key] ?? null;
  // The list's state is for the folder it read; a newer pick waits for the
  // re-read the save starts.
  const state: CloneState | "checking" = !path
    ? "notSet"
    : repo.clonePath === path
      ? repo.clone
      : "checking";

  const choose = async () => {
    if (!settings) return;
    try {
      const picked = await pickFolder();
      if (picked && picked !== path) {
        void save({ clonePaths: withClone(clones, key, picked, key) });
      }
    } catch (e) {
      toastError("Could not open the folder picker", toIpcError(e).message);
    }
  };

  const Icon = state === "found" ? CloneIcon : CloneMissingIcon;
  return (
    <div className={own.clone} data-slot="clone">
      {path ? (
        <span className={own.cloneLine}>
          <span className={styles.cardDetail} title={path}>
            {shortPath(path)}
          </span>
          <span className={own.state} data-state={state}>
            {state !== "checking" && <Icon className={own.stateIcon} />}
            {state === "checking" ? "Checking…" : CLONE_STATE_LABEL[state]}
          </span>
        </span>
      ) : (
        <span className={own.muted}>Not set</span>
      )}
      <span className={styles.cardActions}>
        <button
          type="button"
          disabled={!settings}
          aria-label={`Choose the local clone of ${key}`}
          onClick={() => void choose()}
        >
          Choose…
        </button>
        {path && state === "found" && (
          <button
            type="button"
            aria-label={`Show the clone of ${key} in ${fileManagerName}`}
            title={`Show in ${fileManagerName}`}
            onClick={() => void revealRepositoryClone(key)}
          >
            Show
          </button>
        )}
        {path && (
          <button
            type="button"
            aria-label={`Clear the local clone of ${key}`}
            onClick={() => void save({ clonePaths: withoutClone(clones, key) })}
          >
            Clear
          </button>
        )}
      </span>
    </div>
  );
}

/**
 * A repository Argus has no review of yet, by hand: its key and its clone
 * (the old "Add clone"). It joins the list once saved.
 */
function AddRepository() {
  const settings = useBackendSettingsStore((s) => s.settings);
  const save = useBackendSettingsStore((s) => s.save);
  const [draft, setDraft] = useState<{ repo: string; path: string } | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const clones = settings?.clonePaths ?? {};

  const edit = (next: { repo: string; path: string } | null) => {
    setDraft(next);
    setError(null);
  };
  const choose = async () => {
    if (!draft) return;
    try {
      const path = await pickFolder();
      if (path) setDraft({ ...draft, path });
    } catch (e) {
      setError(toIpcError(e).message);
    }
  };
  const submit = () => {
    if (!draft) return;
    const repo = normalizeRepo(draft.repo);
    const problem = cloneError(repo, draft.path, clones, null);
    if (problem) {
      setError(problem);
      return;
    }
    edit(null);
    void save({ clonePaths: withClone(clones, repo, draft.path, null) });
  };

  if (!draft) {
    return (
      <div className={styles.editActions}>
        <button
          type="button"
          disabled={!settings}
          onClick={() => edit({ repo: "", path: "" })}
        >
          Add repository
        </button>
      </div>
    );
  }
  return (
    <form
      className={styles.card}
      aria-label="New repository"
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <span className={styles.cardName}>New repository</span>
      <div className={styles.field}>
        <label htmlFor="clone-repo">Repository</label>
        <input
          id="clone-repo"
          type="text"
          value={draft.repo}
          placeholder="github.com/acme/widgets"
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          onChange={(e) => setDraft({ ...draft, repo: e.target.value })}
        />
      </div>
      <div className={styles.field}>
        <label htmlFor="clone-path">Local clone</label>
        <div className={styles.fieldRow}>
          <input
            id="clone-path"
            type="text"
            readOnly
            value={draft.path}
            placeholder="Not chosen"
            title={draft.path}
          />
          <button type="button" onClick={() => void choose()}>
            Choose…
          </button>
        </div>
      </div>
      {error && (
        <p className={styles.error} role="alert">
          {error}
        </p>
      )}
      <div className={styles.editActions}>
        <button type="button" onClick={() => edit(null)}>
          Cancel
        </button>
        <button type="submit" className="primary">
          Save
        </button>
      </div>
    </form>
  );
}
