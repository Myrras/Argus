import { describe, expect, it } from "vitest";

import {
  cloneError,
  normalizeRepo,
  withClone,
  withoutClone,
} from "./clonePaths";

describe("clonePaths", () => {
  it("tidies a pasted URL into host/repo without changing its case", () => {
    expect(normalizeRepo(" https://github.com/Acme/Widgets.git/ ")).toBe(
      "github.com/Acme/Widgets",
    );
    expect(normalizeRepo("gitlab.example.test/grp/sub/proj")).toBe(
      "gitlab.example.test/grp/sub/proj",
    );
  });

  it("explains what is missing or wrong", () => {
    const existing = { "github.com/acme/widgets": "/src/widgets" };
    expect(cloneError("", "/x", existing, null)).toMatch(/Enter a repository/);
    expect(cloneError("acme/widgets", "/x", existing, null)).toMatch(
      /host\/owner\/name/,
    );
    expect(cloneError("github.com/acme/widgets", "/x", existing, null)).toBe(
      "github.com/acme/widgets already has a clone.",
    );
    expect(
      cloneError(
        "github.com/acme/widgets",
        "/x",
        existing,
        "github.com/acme/widgets",
      ),
    ).toBeNull();
    expect(cloneError("github.com/acme/other", " ", existing, null)).toBe(
      "Choose a folder.",
    );
  });

  it("adds, renames and removes rows", () => {
    const one = withClone({}, "github.com/a/b", " /src/b ", null);
    expect(one).toEqual({ "github.com/a/b": "/src/b" });
    const renamed = withClone(
      one,
      "github.com/a/c",
      "/src/c",
      "github.com/a/b",
    );
    expect(renamed).toEqual({ "github.com/a/c": "/src/c" });
    expect(withoutClone(renamed, "github.com/a/c")).toEqual({});
  });
});
