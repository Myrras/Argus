import { describe, expect, it } from "vitest";

import {
  appName,
  normalizeExtension,
  overrideError,
  withOverride,
  withoutOverride,
} from "./editorOverrides";

const TYPORA = "/Applications/Typora.app";

describe("editor overrides", () => {
  it("normalizes extensions like the core", () => {
    expect(normalizeExtension("  .MD ")).toBe("md");
    expect(normalizeExtension("..Txt")).toBe("txt");
    expect(normalizeExtension("yml")).toBe("yml");
  });

  it("validates the extension and the application", () => {
    expect(overrideError("", TYPORA, {}, null)).toMatch(/Enter an extension/);
    expect(overrideError("tar.gz", TYPORA, {}, null)).toMatch(/single/);
    expect(overrideError("m d", TYPORA, {}, null)).toMatch(/single/);
    expect(overrideError("md", "  ", {}, null)).toBe("Choose an application.");
    expect(overrideError("md", TYPORA, {}, null)).toBeNull();
  });

  it("refuses a second editor for an extension, but not the row itself", () => {
    const existing = { md: TYPORA };
    expect(overrideError("md", TYPORA, existing, null)).toBe(
      ".md already has an editor.",
    );
    expect(overrideError("md", "/Applications/Other.app", existing, "md")).toBe(
      null,
    );
  });

  it("adds, renames and removes rows", () => {
    const added = withOverride({}, "md", ` ${TYPORA} `, null);
    expect(added).toEqual({ md: TYPORA });
    const renamed = withOverride(added, "markdown", TYPORA, "md");
    expect(renamed).toEqual({ markdown: TYPORA });
    expect(withoutOverride(renamed, "markdown")).toEqual({});
    // The inputs are left alone.
    expect(added).toEqual({ md: TYPORA });
  });

  it("names applications without their path and extension", () => {
    expect(appName(TYPORA)).toBe("Typora");
    expect(appName("C:\\Program Files\\Notepad++\\notepad++.exe")).toBe(
      "notepad++",
    );
    expect(appName("/usr/bin/vim")).toBe("vim");
  });
});
