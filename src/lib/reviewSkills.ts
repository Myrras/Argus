/**
 * Settings → Repositories → Review skill: which code-review skill a
 * repository's AI runs follow (`Settings.reviewSkills`, keyed `host/repo`
 * like `clonePaths`; no entry is `auto`). Pure rules for
 * `ReviewSkillPicker`.
 */

import type {
  ReviewSkill,
  ReviewSkillChoice,
  ReviewSkillList,
  Settings,
  SkillSource,
} from "../ipc/types";

export const AUTO: ReviewSkillChoice = { kind: "auto" };
export const NONE: ReviewSkillChoice = { kind: "none" };

/** The choice stored for `repo` (`auto` when none is). */
export function choiceFor(
  settings: Settings | null,
  repo: string,
): ReviewSkillChoice {
  return settings?.reviewSkills?.[repo] ?? AUTO;
}

export function sameChoice(a: ReviewSkillChoice, b: ReviewSkillChoice) {
  if (a.kind !== b.kind) return false;
  return (
    a.kind !== "skill" ||
    (b.kind === "skill" && a.source === b.source && a.name === b.name)
  );
}

/** The choices with `repo` set to `choice` (`auto` drops the entry). */
export function withChoice(
  existing: Record<string, ReviewSkillChoice> | undefined,
  repo: string,
  choice: ReviewSkillChoice,
): Record<string, ReviewSkillChoice> {
  const next = { ...(existing ?? {}) };
  if (choice.kind === "auto") delete next[repo];
  else next[repo] = choice;
  return next;
}

/** Where a skill comes from, as a group heading. */
export const SOURCE_GROUP: Record<SkillSource, string> = {
  user: "Your skills",
  plugin: "Plugin skills",
  project: "This project",
};

/** Where a skill comes from, after its name. */
export const SOURCE_LABEL: Record<SkillSource, string> = {
  user: "yours",
  plugin: "plugin",
  project: "project",
};

/** The Auto row's label: what Auto would pick for this repository now. */
export function autoLabel(list: ReviewSkillList | null): string {
  if (!list) return "Auto";
  if (list.autoPick) return `Auto (project's own: ${list.autoPick})`;
  return list.projectListed
    ? "Auto (none found)"
    : "Auto (project's own, checked at run time)";
}

/** The Auto row's description. */
export function autoHelp(list: ReviewSkillList | null): string {
  if (list && !list.projectListed) {
    return "Without a local clone, Argus looks for .claude/skills/code-review on the target branch when a run starts.";
  }
  return "The project's own review skill, read from the target branch.";
}

/** The current choice in full, for the line under the repository's key. */
export function choiceSummary(
  choice: ReviewSkillChoice,
  list: ReviewSkillList | null,
): string {
  switch (choice.kind) {
    case "auto":
      if (!list) return "Auto";
      if (list.autoPick) return `Auto: ${list.autoPick}`;
      return list.projectListed
        ? "Auto: none found"
        : "Auto: project's own, checked at run time";
    case "none":
      return "None";
    case "skill":
      return `${choice.name} · ${SOURCE_LABEL[choice.source]}`;
  }
}

/**
 * The skills matching `query` (name, description or source, any case),
 * grouped by source in the list's order.
 */
export function filterSkills(
  skills: ReviewSkill[],
  query: string,
): { source: SkillSource; skills: ReviewSkill[] }[] {
  const q = query.trim().toLowerCase();
  const hit = (s: ReviewSkill) =>
    !q ||
    s.name.toLowerCase().includes(q) ||
    (s.description ?? "").toLowerCase().includes(q) ||
    SOURCE_GROUP[s.source].toLowerCase().includes(q);
  const order: SkillSource[] = ["user", "plugin", "project"];
  return order
    .map((source) => ({
      source,
      skills: skills.filter((s) => s.source === source && hit(s)),
    }))
    .filter((g) => g.skills.length > 0);
}
