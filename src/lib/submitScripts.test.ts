import { describe, expect, it } from "vitest";

import type {
  ScriptDraftProgress,
  ScriptResult,
  ScriptsDone,
} from "../ipc/types";
import { scriptDraft } from "../test/fixtures";
import {
  attachedLabel,
  draftStatusLabel,
  formatElapsed,
  progressSnippet,
  stageLabel,
  withProgress,
  latestScript,
  requestCount,
  upsertDraft,
  doneToasts,
  insideDir,
  isTicked,
  moveScript,
  newScript,
  parseArgs,
  parseRepos,
  scriptMatches,
  scriptName,
  scriptsFor,
  slugFileName,
  tailLines,
} from "./submitScripts";

const REPO = "gitlab.example.test/acme-fixture/api";

function result(overrides: Partial<ScriptResult> = {}): ScriptResult {
  return {
    scriptId: "s1",
    name: "Tick the checklist",
    status: "ok",
    exitCode: 0,
    stdout: "",
    stderr: "",
    durationMs: 10,
    message: null,
    finishedAt: "2026-10-23T10:00:00Z",
    ...overrides,
  };
}

describe("submitScripts", () => {
  it("offers a script for its verdicts and repositories only", () => {
    const any = newScript("a", { verdicts: ["approve", "requestChanges"] });
    expect(scriptMatches(any, "approve", REPO)).toBe(true);
    expect(scriptMatches(any, "requestChanges", "github.com/x/y")).toBe(true);
    expect(scriptMatches(any, "comment", REPO)).toBe(false);
    // Case and a trailing slash do not count, as in the core.
    const one = newScript("b", {
      repos: ["GitLab.example.test/Acme-Fixture/API/"],
    });
    expect(scriptMatches(one, "approve", REPO)).toBe(true);
    expect(scriptMatches(one, "approve", "github.com/acme-fixture/api")).toBe(
      false,
    );
    // No verdict: never offered.
    expect(
      scriptMatches(newScript("c", { verdicts: [] }), "approve", REPO),
    ).toBe(false);
  });

  it("lists the offered scripts in their run order, none without a verdict", () => {
    const a = newScript("a");
    const b = newScript("b", { verdicts: ["comment"] });
    const c = newScript("c");
    expect(scriptsFor([a, b, c], "approve", REPO).map((s) => s.id)).toEqual([
      "a",
      "c",
    ]);
    expect(scriptsFor([a, b, c], null, REPO)).toEqual([]);
  });

  it("ticks by default until the user ticks otherwise", () => {
    const on = newScript("on", { defaultOn: true });
    const off = newScript("off");
    expect(isTicked(on, {})).toBe(true);
    expect(isTicked(off, {})).toBe(false);
    expect(isTicked(on, { on: false })).toBe(false);
    expect(isTicked(off, { off: true })).toBe(true);
  });

  it("reads arguments one per line and repositories by line or comma", () => {
    expect(parseArgs("--quiet\n\n  two words \n")).toEqual([
      "--quiet",
      "  two words ",
    ]);
    expect(
      parseRepos(`${REPO}/, github.com/a/b\n${REPO.toUpperCase()}`),
    ).toEqual([REPO, "github.com/a/b"]);
  });

  it("moves a script within bounds", () => {
    const list = [newScript("a"), newScript("b"), newScript("c")];
    expect(moveScript(list, "c", -1).map((s) => s.id)).toEqual(["a", "c", "b"]);
    expect(moveScript(list, "a", -1).map((s) => s.id)).toEqual(["a", "b", "c"]);
  });

  it("names a script by its name, else its command's file", () => {
    expect(scriptName({ name: " Tick ", command: "/x/y.sh" })).toBe("Tick");
    expect(scriptName({ name: "", command: "/x/y.sh" })).toBe("y.sh");
    expect(scriptName({ name: "", command: "" })).toBe("Untitled script");
  });

  it("knows a script Argus saved by its folder", () => {
    const dir = "/Users/fixture/Library/Argus/scripts";
    expect(insideDir(`${dir}/tick.sh`, dir)).toBe(true);
    expect(insideDir(`${dir}/`, dir)).toBe(false);
    expect(insideDir(`${dir}-other/tick.sh`, dir)).toBe(false);
    expect(insideDir("/Users/fixture/bin/tick.sh", dir)).toBe(false);
    expect(insideDir(`${dir}/tick.sh`, null)).toBe(false);
  });

  it("slugs a file name as the core does", () => {
    expect(slugFileName("Tick the checklist!")).toBe("tick-the-checklist.sh");
    expect(slugFileName("  tick.sh ")).toBe("tick.sh");
    expect(slugFileName("--")).toBe("");
    expect(slugFileName("a".repeat(80))).toBe(`${"a".repeat(64)}.sh`);
  });

  it("keeps the last lines of output", () => {
    expect(tailLines("one\ntwo\n\nthree\nfour\n", 2)).toBe("three\nfour");
  });

  it("toasts failures with their stderr, success briefly, skips quietly", () => {
    const done: ScriptsDone = {
      reviewId: "r1",
      results: [
        result(),
        result({
          scriptId: "s2",
          name: "Notify",
          status: "failed",
          exitCode: 2,
          stderr: "a\nb\nc\nd\n",
          message: "Exited with status 2.",
        }),
        result({
          scriptId: "s3",
          name: "Gone",
          status: "notExecutable",
          message: "/x is not executable — run chmod +x /x",
        }),
        result({
          scriptId: "s4",
          name: "Elsewhere",
          status: "skipped",
          message: "Not set to run for x.",
        }),
      ],
    };
    expect(doneToasts(done)).toEqual([
      {
        kind: "error",
        message: 'Script "Notify": failed',
        detail: "Exited with status 2.\nb\nc\nd",
      },
      {
        kind: "error",
        message: 'Script "Gone": not executable',
        detail: "/x is not executable — run chmod +x /x",
      },
      { kind: "success", message: "Tick the checklist ran" },
      {
        kind: "info",
        message: "Skipped Elsewhere",
        detail: "Not set to run for x.",
      },
    ]);
    expect(
      doneToasts({ reviewId: "r1", results: [result(), result()] }),
    ).toEqual([{ kind: "success", message: "Ran 2 scripts" }]);
  });
});

describe("Write with AI drafts", () => {
  const script = {
    name: "Tick",
    verdicts: ["approve" as const],
    script: "#!/bin/sh\n",
    explanation: "",
  };

  it("keeps the newer copy, whatever order they arrive in", () => {
    const d = scriptDraft({ revision: 5 });
    const older = { ...d, revision: 4, status: "idle" as const };
    const newer = { ...d, revision: 6, status: "failed" as const };
    expect(upsertDraft([], d)).toEqual([d]);
    expect(upsertDraft([d], older)).toEqual([d]);
    expect(upsertDraft([d], newer)).toEqual([newer]);
  });

  it("reads the newest script and the status in words", () => {
    const d = scriptDraft({ base: script });
    expect(latestScript(d)).toBe(script);
    const answered = {
      ...d,
      turns: [{ ...d.turns[0]!, result: { ...script, name: "New" } }],
      status: "idle" as const,
    };
    expect(latestScript(answered)?.name).toBe("New");
    expect(draftStatusLabel(answered)).toBe("Ready");
    expect(draftStatusLabel(d)).toBe("Writing…");
    expect(draftStatusLabel({ ...d, waiting: true })).toBe(
      "Waiting for another draft…",
    );
    expect(draftStatusLabel({ ...d, status: "cancelled" })).toBe("Stopped");
    expect(
      draftStatusLabel({ ...d, status: "failed", error: "No agent." }),
    ).toBe("Failed: No agent.");
    expect(requestCount(d)).toBe("1 request");
    expect(requestCount({ ...d, turns: [...d.turns, ...d.turns] })).toBe(
      "2 requests",
    );
  });
});

describe("draft progress", () => {
  const progress = (over: Partial<ScriptDraftProgress> = {}) => ({
    turn: 0,
    stage: "thinking" as const,
    startedAt: "2026-10-16T10:00:00Z",
    thinking: "",
    thinkingCut: false,
    thinkingTokens: null,
    answerChars: 0,
    notice: null,
    ...over,
  });

  it("applies a report only to the draft running that turn", () => {
    const running = scriptDraft();
    const idle = scriptDraft({ status: "idle" });
    const waiting = scriptDraft({ waiting: true });
    const all = [running, idle, waiting];
    const p = progress({ thinking: "hm" });
    const out = all.map((d) =>
      withProgress(all, { draftId: d.id, progress: p }),
    );
    expect(out[0]![0]!.progress).toEqual(p);
    expect(out[1]![1]!.progress).toBeNull();
    expect(out[2]![2]!.progress).toBeNull();
    // A report for an earlier turn is late: dropped.
    expect(
      withProgress(all, {
        draftId: running.id,
        progress: progress({ turn: 3 }),
      })[0]!.progress,
    ).toBeNull();
  });

  it("reads the stage, the time and the latest thought", () => {
    expect(stageLabel("starting", "claude")).toBe("Starting claude…");
    expect(stageLabel("repairing")).toMatch(/asking again/);
    expect(formatElapsed(4_900)).toBe("4s");
    expect(formatElapsed(65_000)).toBe("1m 05s");
    expect(formatElapsed(3_720_000)).toBe("1h 02m");
    expect(progressSnippet(null)).toBeNull();
    expect(progressSnippet(progress())).toBeNull();
    expect(
      progressSnippet(progress({ thinking: "First.\n\nThen this.\n" })),
    ).toBe("Then this.");
    expect(
      progressSnippet(progress({ thinking: "x", notice: "The API is busy." })),
    ).toBe("The API is busy.");
    expect(progressSnippet(progress({ thinking: "y".repeat(200) }), 10)).toBe(
      `…${"y".repeat(9)}`,
    );
    expect(
      draftStatusLabel(
        scriptDraft({ progress: progress({ stage: "writing" }) }),
      ),
    ).toBe("Writing the script…");
    expect(attachedLabel([])).toBeNull();
    expect(attachedLabel(["MR !1", "a.md"])).toBe("Attached: MR !1 · a.md");
  });
});
