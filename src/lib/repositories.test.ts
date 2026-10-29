import { describe, expect, it } from "vitest";

import type { RepoSetup } from "../ipc/types";
import { settings } from "../test/fixtures";
import {
  filterRepos,
  globalLanguage,
  isSetUp,
  languageFor,
  languageLabel,
  languageOfOption,
  optionOf,
  otherLanguage,
  sameLanguage,
  withLanguage,
} from "./repositories";

const WIDGETS = "github.com/acme-fixture/widgets";

function repo(key: string): RepoSetup {
  return {
    key,
    host: key.split("/")[0],
    repo: key.split("/").slice(1).join("/"),
    provider: "github",
    openReviews: 0,
    clonePath: null,
    clone: "notSet",
    reviewSkill: { kind: "auto" },
    reviewLanguage: null,
  };
}

describe("repositories", () => {
  it("is set up once anything is stored for the repository", () => {
    expect(isSetUp(null, WIDGETS)).toBe(false);
    expect(isSetUp(settings(), WIDGETS)).toBe(false);
    expect(
      isSetUp(settings({ clonePaths: { [WIDGETS]: "/w" } }), WIDGETS),
    ).toBe(true);
    expect(
      isSetUp(
        settings({ reviewSkills: { [WIDGETS]: { kind: "none" } } }),
        WIDGETS,
      ),
    ).toBe(true);
    expect(
      isSetUp(
        settings({ reviewLanguages: { [WIDGETS]: { kind: "auto" } } }),
        WIDGETS,
      ),
    ).toBe(true);
  });

  it("filters by key, any case", () => {
    const all = [repo(WIDGETS), repo("gitlab.example.test/acme-fixture/api")];
    expect(filterRepos(all, "")).toBe(all);
    expect(filterRepos(all, " GITLAB ").map((r) => r.key)).toEqual([
      "gitlab.example.test/acme-fixture/api",
    ]);
  });

  it("names, compares and stores languages", () => {
    expect(languageLabel(null)).toBe("Auto");
    expect(languageLabel({ kind: "dutch" })).toBe("Nederlands");
    expect(languageLabel({ kind: "other", name: "Deutsch" })).toBe("Deutsch");
    expect(globalLanguage(settings())).toEqual({ kind: "auto" });
    expect(
      globalLanguage(
        settings({
          ai: { ...settings().ai, reviewLanguage: { kind: "english" } },
        }),
      ),
    ).toEqual({ kind: "english" });
    expect(languageFor(settings(), WIDGETS)).toBeNull();
    expect(sameLanguage(null, null)).toBe(true);
    expect(sameLanguage(null, { kind: "auto" })).toBe(false);
    expect(
      sameLanguage({ kind: "other", name: "a" }, { kind: "other", name: "b" }),
    ).toBe(false);
    const set = withLanguage(undefined, WIDGETS, { kind: "dutch" });
    expect(set).toEqual({ [WIDGETS]: { kind: "dutch" } });
    expect(withLanguage(set, WIDGETS, null)).toEqual({});
  });

  it("maps the picker's options and tidies a typed name like the core", () => {
    expect(optionOf(null)).toBe("default");
    expect(optionOf({ kind: "other", name: "x" })).toBe("other");
    expect(languageOfOption("default")).toBeNull();
    expect(languageOfOption("other")).toBeUndefined();
    expect(languageOfOption("english")).toEqual({ kind: "english" });
    expect(otherLanguage("  Deutsch \n ")).toEqual({
      kind: "other",
      name: "Deutsch",
    });
    expect(otherLanguage("   ")).toBeNull();
    expect(otherLanguage("x".repeat(60))).toEqual({
      kind: "other",
      name: "x".repeat(40),
    });
  });
});
