/**
 * Settings → Repositories → Review skill: per repository, the code-review
 * skill whose criteria the AI pass applies (`Settings.reviewSkills`, keyed
 * `host/repo` like the local clones). `ReviewSkillField` sits in the
 * repository's card: the current choice in full and "Change", which opens
 * the picker inside the card: a filter over a scrolling list with Auto and
 * None on top, then the skills whose name says "review" by source (yours,
 * plugins, the project's), each with its description. Picking one applies
 * it at once (the family's model for choices) and closes the picker.
 *
 * Arrow keys move between the choices (from the filter too); the choices
 * are a radio group.
 */

import { useEffect, useRef, useState, type KeyboardEvent } from "react";

import type {
  ReviewSkill,
  ReviewSkillChoice,
  ReviewSkillList,
} from "../ipc/types";
import {
  AUTO,
  NONE,
  SOURCE_GROUP,
  SOURCE_LABEL,
  autoHelp,
  autoLabel,
  choiceFor,
  choiceSummary,
  filterSkills,
  sameChoice,
  withChoice,
} from "../lib/reviewSkills";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { useReviewSkillsStore } from "../state/reviewSkillsStore";
import { CheckIcon } from "./icons";
import own from "./ReviewSkillPicker.module.css";
import styles from "./SettingsModal.module.css";

/** The review skill of `repo`: its choice, and the picker on "Change". */
export function ReviewSkillField({ repo }: { repo: string }) {
  const settings = useBackendSettingsStore((s) => s.settings);
  const save = useBackendSettingsStore((s) => s.save);
  const load = useReviewSkillsStore((s) => s.load);
  const list = useReviewSkillsStore((s) => s.lists[repo] ?? null);
  const [open, setOpen] = useState(false);
  // Listed once when the card shows: the summary says what Auto picks
  // without opening.
  useEffect(() => {
    void load(repo);
  }, [repo, load]);

  const choice = choiceFor(settings, repo);
  const choose = (next: ReviewSkillChoice) => {
    setOpen(false);
    if (!settings || sameChoice(next, choiceFor(settings, repo))) return;
    void save({
      reviewSkills: withChoice(settings.reviewSkills, repo, next),
    });
  };

  return (
    <div className={own.field} data-slot="review-skill">
      <div className={own.fieldHeader}>
        {/* Wrapping, not truncated: the skill is read in full. */}
        <span
          className={`${styles.cardDetail} ${styles.cardDetailWrap}`}
          data-slot="skill-choice"
        >
          {choiceSummary(choice, list)}
        </span>
        <button
          type="button"
          className={styles.pushRight}
          aria-expanded={open}
          disabled={!settings}
          aria-label={
            open
              ? `Close the review skill picker of ${repo}`
              : `Choose the review skill of ${repo}`
          }
          onClick={() => {
            if (!open) void load(repo);
            setOpen(!open);
          }}
        >
          {open ? "Done" : "Change"}
        </button>
      </div>
      {open && (
        <Picker repo={repo} choice={choice} list={list} onChoose={choose} />
      )}
    </div>
  );
}

function Picker({
  repo,
  choice,
  list,
  onChoose,
}: {
  repo: string;
  choice: ReviewSkillChoice;
  list: ReviewSkillList | null;
  onChoose: (choice: ReviewSkillChoice) => void;
}) {
  const loading = useReviewSkillsStore((s) => !!s.loading[repo]);
  const error = useReviewSkillsStore((s) => s.errors[repo] ?? null);
  const [query, setQuery] = useState("");
  const listRef = useRef<HTMLDivElement>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  useEffect(() => filterRef.current?.focus(), []);

  const skills = list?.skills ?? [];
  const groups = filterSkills(skills, query);
  // A chosen skill that is not installed any more: still shown, marked.
  const gone =
    choice.kind === "skill" &&
    list !== null &&
    !skills.some((s) => s.source === choice.source && s.name === choice.name)
      ? choice
      : null;

  const radios = () =>
    Array.from(
      listRef.current?.querySelectorAll<HTMLElement>('[role="radio"]') ?? [],
    );
  const move = (e: KeyboardEvent, from: HTMLElement | null) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const all = radios();
    const at = from ? all.indexOf(from) : -1;
    const next = e.key === "ArrowDown" ? at + 1 : at - 1;
    if (next < 0) filterRef.current?.focus();
    else all[Math.min(next, all.length - 1)]?.focus();
  };

  return (
    <div className={own.picker}>
      <input
        ref={filterRef}
        type="search"
        value={query}
        placeholder="Filter skills"
        aria-label={`Filter the review skills of ${repo}`}
        autoComplete="off"
        autoCorrect="off"
        autoCapitalize="off"
        spellCheck={false}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => move(e, null)}
      />
      <div
        ref={listRef}
        role="radiogroup"
        aria-label={`Review skill for ${repo}`}
        aria-busy={loading}
        className={own.list}
        onKeyDown={(e) => move(e, e.target as HTMLElement)}
      >
        <Option
          name={autoLabel(list)}
          help={autoHelp(list)}
          checked={choice.kind === "auto"}
          onSelect={() => onChoose(AUTO)}
        />
        <Option
          name="None"
          help="Argus' own review instructions only."
          checked={choice.kind === "none"}
          onSelect={() => onChoose(NONE)}
        />
        {gone && (
          <Option
            name={gone.name}
            skill
            help={`Not found any more (${SOURCE_LABEL[gone.source]}): runs use Auto until you pick another.`}
            warn
            checked
            onSelect={() => onChoose(gone)}
          />
        )}
        <div className={own.divider} aria-hidden />
        {groups.map((g) => (
          <div
            key={g.source}
            className={own.group}
            role="group"
            aria-label={SOURCE_GROUP[g.source]}
          >
            <span className={own.groupLabel} aria-hidden>
              {SOURCE_GROUP[g.source]}
            </span>
            {g.skills.map((s) => (
              <SkillOption
                key={`${s.source}:${s.name}`}
                skill={s}
                checked={
                  choice.kind === "skill" &&
                  choice.source === s.source &&
                  choice.name === s.name
                }
                onSelect={() =>
                  onChoose({ kind: "skill", source: s.source, name: s.name })
                }
              />
            ))}
          </div>
        ))}
        {error ? (
          <p className={own.note} role="alert">
            Couldn&apos;t list the skills: {error}
          </p>
        ) : list === null ? (
          <p className={own.note}>Looking for skills…</p>
        ) : skills.length === 0 ? (
          <p className={own.note}>
            No installed skill has &ldquo;review&rdquo; in its name.
          </p>
        ) : (
          groups.length === 0 && (
            <p className={own.note}>
              No skill matches &ldquo;{query.trim()}&rdquo;.
            </p>
          )
        )}
      </div>
    </div>
  );
}

function SkillOption({
  skill,
  checked,
  onSelect,
}: {
  skill: ReviewSkill;
  checked: boolean;
  onSelect: () => void;
}) {
  return (
    <Option
      name={skill.name}
      skill
      help={skill.description}
      label={`${skill.name}, ${SOURCE_LABEL[skill.source]}`}
      checked={checked}
      onSelect={onSelect}
    />
  );
}

function Option({
  name,
  help,
  label,
  skill = false,
  warn = false,
  checked,
  onSelect,
}: {
  name: string;
  help?: string | null;
  /** The accessible name, when it says more than `name`. */
  label?: string;
  /** The name is a skill's identifier (monospace). */
  skill?: boolean;
  warn?: boolean;
  checked: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={checked}
      aria-label={label ?? name}
      title={help ?? undefined}
      className={own.option}
      onClick={onSelect}
    >
      <span className={own.mark} aria-hidden>
        {checked && <CheckIcon size={13} />}
      </span>
      <span className={own.body}>
        <span className={`${own.name} ${skill ? own.skillName : ""}`}>
          {name}
        </span>
        {help && (
          <span className={`${own.help} ${warn ? own.gone : ""}`}>{help}</span>
        )}
      </span>
    </button>
  );
}
