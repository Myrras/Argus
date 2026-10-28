import { describe, expect, it } from "vitest";

import type { ReviewSkill, ReviewSkillList } from "../ipc/types";
import { settings } from "../test/fixtures";
import {
  AUTO,
  NONE,
  autoLabel,
  choiceFor,
  choiceSummary,
  filterSkills,
  knownRepos,
  sameChoice,
  withChoice,
} from "./reviewSkills";

const WIDGETS = "github.com/acme-fixture/widgets";

const skills: ReviewSkill[] = [
  { name: "code-review", description: "My own review.", source: "user" },
  {
    name: "fixture-tools:pr-review",
    description: "Reviews pull requests for SQL.",
    source: "plugin",
  },
  { name: "code-review", description: null, source: "project" },
];

const list = (over: Partial<ReviewSkillList> = {}): ReviewSkillList => ({
  skills,
  autoPick: "code-review",
  projectListed: true,
  ...over,
});

describe("review skill choices", () => {
  it("defaults to auto and drops auto from the stored map", () => {
    expect(choiceFor(null, WIDGETS)).toEqual(AUTO);
    expect(choiceFor(settings(), WIDGETS)).toEqual(AUTO);
    const pick = {
      kind: "skill",
      source: "plugin",
      name: "fixture-tools:pr-review",
    } as const;
    const set = withChoice(undefined, WIDGETS, pick);
    expect(set).toEqual({ [WIDGETS]: pick });
    expect(choiceFor(settings({ reviewSkills: set }), WIDGETS)).toEqual(pick);
    expect(withChoice(set, WIDGETS, NONE)).toEqual({ [WIDGETS]: NONE });
    expect(withChoice(set, WIDGETS, AUTO)).toEqual({});
    expect(sameChoice(pick, { ...pick })).toBe(true);
    expect(sameChoice(pick, { ...pick, source: "user" })).toBe(false);
    expect(sameChoice(AUTO, NONE)).toBe(false);
  });

  it("says what auto picks, or that it is not known yet", () => {
    expect(autoLabel(list())).toBe("Auto (project's own: code-review)");
    expect(autoLabel(list({ autoPick: null }))).toBe("Auto (none found)");
    expect(autoLabel(list({ autoPick: null, projectListed: false }))).toBe(
      "Auto (project's own, checked at run time)",
    );
    expect(autoLabel(null)).toBe("Auto");
    expect(choiceSummary(NONE, list())).toBe("None");
    expect(
      choiceSummary(
        { kind: "skill", source: "user", name: "code-review" },
        null,
      ),
    ).toBe("code-review (yours)");
  });

  it("filters by name, description and source, grouped by source", () => {
    expect(filterSkills(skills, "").map((g) => g.source)).toEqual([
      "user",
      "plugin",
      "project",
    ]);
    expect(filterSkills(skills, "SQL")).toEqual([
      { source: "plugin", skills: [skills[1]] },
    ]);
    expect(filterSkills(skills, "project")).toEqual([
      { source: "project", skills: [skills[2]] },
    ]);
    expect(filterSkills(skills, "nothing like it")).toEqual([]);
  });

  it("lists every repository with a clone, a choice or a review, once", () => {
    const s = settings({
      clonePaths: { [WIDGETS]: "/src/widgets" },
      reviewSkills: { "gitlab.example.test/acme-fixture/api": NONE },
    });
    expect(
      knownRepos(s, [
        { host: "github.com", repo: "acme-fixture/widgets" },
        { host: "github.com", repo: "acme-fixture/gadgets" },
      ]),
    ).toEqual([
      "github.com/acme-fixture/gadgets",
      WIDGETS,
      "gitlab.example.test/acme-fixture/api",
    ]);
  });
});
