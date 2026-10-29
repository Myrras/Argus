/**
 * The TS half of the wire-shape check (the Rust half is
 * `crates/argus-core/tests/wire_shapes.rs`): fully filled literals typed
 * against `types.ts` must have exactly the key paths, and the enums exactly
 * the values, that `wire-shapes.json` records for the Rust types. The type
 * annotations keep the literals honest (no unknown or missing keys), the
 * comparison keeps them in step with the backend.
 */

import { describe, expect, it } from "vitest";

import snapshot from "./wire-shapes.json";
import type {
  Account,
  AccountState,
  AiRunState,
  AiRunStatus,
  ChecklistItem,
  CodeSnapshot,
  CommentableLines,
  DiffKind,
  FileStatus,
  Finding,
  FindingPostResult,
  LineChange,
  LineStatus,
  CloneState,
  CloneStatus,
  CloneSuggestion,
  LocalFile,
  NewComment,
  PostedNote,
  PostErrorKind,
  OutsideDiff,
  PostResult,
  ProviderId,
  RefreshReport,
  ReviewRefresh,
  RepoSetup,
  ReviewLanguage,
  ReviewSkillChoice,
  ReviewSkillList,
  RepeatReason,
  ReviewFiles,
  ReviewRecord,
  ReviewVerdict,
  MyReviewState,
  ReviewState,
  ReviewStatus,
  RoundReason,
  GeneratedScript,
  DraftStage,
  ScriptDraft,
  ScriptDraftProgress,
  ScriptDraftProgressEvent,
  ScriptDraftStatus,
  ScriptGenerationStatus,
  ScriptResult,
  ScriptsDone,
  ScriptStatus,
  Settings,
  CacheStats,
  Severity,
  Side,
  SkillSource,
  SnapshotSource,
  StoredThread,
  ThreadHistory,
  TimelineEntry,
  TimelineEvent,
  Verdict,
  VerdictLabel,
} from "./types";

/** Every key path of `v`, as `shape()` in wire_shapes.rs. */
function shape(v: unknown): string[] {
  const out = new Set<string>();
  const walk = (value: unknown, prefix: string) => {
    if (typeof value !== "object" || value === null || Array.isArray(value))
      return;
    for (const [k, child] of Object.entries(value)) {
      const path = prefix + k;
      out.add(path);
      if (Array.isArray(child)) {
        const first: unknown = child[0];
        if (
          typeof first === "object" &&
          first !== null &&
          !Array.isArray(first)
        )
          walk(first, `${path}[].`);
      } else if (typeof child === "object" && child !== null) {
        walk(child, `${path}.`);
      }
    }
  };
  walk(v, "");
  return [...out].sort();
}

/** True only when `A` lists every member of `T` (checked by `tsc`). */
type Covers<T, A extends readonly unknown[]> = [Exclude<T, A[number]>] extends [
  never,
]
  ? true
  : false;

const T = "2026-10-10T09:00:00Z";
const REVIEW = "00000000-0000-4000-8000-000000000001";
const THREAD = "00000000-0000-4000-8000-000000000002";
const FINDING = "00000000-0000-4000-8000-000000000003";

const record: ReviewRecord = {
  id: REVIEW,
  provider: "github",
  host: "github.com",
  repo: "acme-fixture/widgets",
  number: 12,
  url: "https://github.com/acme-fixture/widgets/pull/12",
  title: "Add widget caching",
  author: "alice-fixture",
  updatedAt: T,
  state: "open",
  draft: false,
  headSha: "c2",
  baseSha: "b0",
  lastReviewedSha: "c1",
  lastReviewedAt: T,
  approvedAt: T,
  round: 2,
  roundReasons: ["newCommits"],
  status: "needsReReview",
  untouchedThreads: 1,
  requested: true,
  rerequestedAt: T,
  myReviewState: "requestedChanges",
  myReviewedAt: T,
  myReviewedSha: "c1",
  listed: true,
  viewer: "rev-fixture",
  fetchedAt: T,
};

const thread: StoredThread = {
  threadId: THREAD,
  reviewId: REVIEW,
  mine: true,
  startedByMe: true,
  id: "4001",
  author: "rev-fixture",
  createdAt: T,
  resolved: false,
  resolvable: true,
  outdated: true,
  anchor: {
    pathOld: "src/cache.rs",
    pathNew: "src/cache.rs",
    lineOld: 9,
    lineNew: 10,
    startLine: 9,
    side: "new",
    baseSha: "b0",
    startSha: "b0",
    headSha: "c1",
    diffHunk: "@@ -1 +1 @@",
    currentLine: 10,
    outsideDiff: false,
  },
  notes: [
    {
      id: "n1",
      author: "rev-fixture",
      body: "Why?",
      createdAt: T,
      system: false,
    },
  ],
};

const files: ReviewFiles = {
  reviewId: REVIEW,
  baseSha: "b0",
  headSha: "c2",
  sinceLastReview: false,
  truncated: false,
  baseApproximate: false,
  reviewBaseSha: "b0",
  files: [
    {
      path: "src/cache.rs",
      oldPath: "src/old.rs",
      status: "renamed",
      additions: 1,
      deletions: 1,
      patch: "@@ -1 +1 @@",
      contentSha: "blob",
      binary: false,
      tooLarge: false,
      viewed: true,
    },
  ],
};

const commentable: CommentableLines = {
  reviewId: REVIEW,
  path: "src/cache.rs",
  baseSha: "b0",
  headSha: "c2",
  hunks: [{ old: [1], new: [1, 2] }],
  outside: "native",
};

const newComment: NewComment = {
  path: "src/cache.rs",
  line: 10,
  side: "new",
  startLine: 9,
  body: "Why?",
  headSha: "c2",
  baseSha: "b0",
};

const posted: PostResult = {
  status: "posted",
  url: "u",
  providerThreadId: "1",
  providerNoteId: "1",
  path: "src/cache.rs",
  side: "new",
  line: 10,
  startLine: 9,
  headSha: "c2",
};

const failed: PostResult = {
  status: "failed",
  kind: "postRejected",
  message: "GitHub: refused",
};

const postedNote: PostedNote = {
  url: "u",
  providerThreadId: "1",
  providerNoteId: "2",
};

const report: RefreshReport = {
  changed: [REVIEW],
  errors: [
    {
      provider: "gitlab",
      host: "gitlab.example.test",
      review: REVIEW,
      message: "GitLab is not set up.",
      unavailable: true,
      rateLimitedUntil: T,
    },
  ],
  refreshedAt: T,
};

const settings: Settings = {
  accounts: { "github:github.com": { enabled: true, account: "work" } },
  pollIntervalSecs: 180,
  ai: {
    enabled: true,
    adapter: "claude",
    args: [],
    binary: "/bin/claude",
    reviewLanguage: { kind: "english" },
  },
  clonePaths: {},
  editorOverrides: {},
  defaultEditor: null,
  openWithRepository: true,
  reviewSkills: {},
  reviewLanguages: {},
  cloneSuggestIgnored: [],
  submitScripts: [
    {
      id: "0f2c4b1e-2f0a-4f7e-9b51-6f0e7c1d2a3b",
      name: "Tick the checklist",
      command: "/Users/fixture/bin/tick.sh",
      args: ["--quiet"],
      verdicts: ["approve"],
      repos: ["gitlab.example.test/acme-fixture/api"],
      defaultOn: true,
      timeoutSecs: 60,
    },
  ],
  cacheLimitMb: 512,
};

const cacheStats: CacheStats = { entries: 3, bytes: 1024 };

const scriptResult: ScriptResult = {
  scriptId: "0f2c4b1e-2f0a-4f7e-9b51-6f0e7c1d2a3b",
  name: "Tick the checklist",
  status: "failed",
  exitCode: 1,
  stdout: "out",
  stderr: "err",
  durationMs: 120,
  message: "Exited with status 1.",
  finishedAt: T,
};

const scriptsDone: ScriptsDone = { reviewId: REVIEW, results: [scriptResult] };

const generatedScript: GeneratedScript = {
  name: "Tick the checklist",
  verdicts: ["approve"],
  script: "#!/bin/sh\nset -eu\n",
  explanation: "Ticks every task.",
};

const scriptDraft: ScriptDraft = {
  id: "5b0c1a2e-8f3d-4c6b-9a71-2e4f6d8c0b13",
  scriptId: "0f2c4b1e-2f0a-4f7e-9b51-6f0e7c1d2a3b",
  title: "Tick the checklist",
  base: generatedScript,
  turns: [
    {
      prompt: "Tick every checklist item",
      at: T,
      result: generatedScript,
      error: "Stopped.",
      attachments: ["MR !12"],
      notes: ["Not read: /etc/hosts (outside your local clones)"],
    },
  ],
  status: "failed",
  waiting: false,
  error: "Couldn't write the script.",
  progress: null as ScriptDraftProgress | null,
  createdAt: T,
  updatedAt: T,
  revision: 3,
};

const draftProgress: ScriptDraftProgress = {
  turn: 0,
  stage: "thinking",
  startedAt: T,
  thinking: "Reading the template.",
  thinkingCut: false,
  thinkingTokens: 150,
  answerChars: 0,
  notice: "The API did not answer.",
};
scriptDraft.progress = draftProgress;

const draftProgressEvent: ScriptDraftProgressEvent = {
  draftId: "5b0c1a2e-8f3d-4c6b-9a71-2e4f6d8c0b13",
  progress: draftProgress,
};

const generationStatus: ScriptGenerationStatus = {
  available: false,
  adapter: "claude",
  reason: "The AI pass is off.",
};

const account: Account = {
  key: "github:github.com",
  provider: "github",
  host: "github.com",
  user: "work",
  state: {
    kind: "notLoggedIn",
    message: "gh's login of work on github.com is no longer valid.",
  },
  active: "rev-fixture",
  accounts: ["rev-fixture", "work"],
  account: "work",
  enabled: true,
  scheme: "https",
  loginCommand: "gh auth login --hostname github.com",
};

const lineStatuses: LineStatus[] = [
  {
    kind: "located",
    path: "src/cache.rs",
    startLine: 9,
    line: 10,
    moved: true,
    renamed: false,
    touched: true,
  },
  { kind: "deleted", path: "src/cache.rs", removedBy: "c2" },
  { kind: "fileGone", path: "src/old.rs", removedBy: "c2" },
  { kind: "unlocated", reason: "rebased" },
  { kind: "notCode" },
];

const timeline: TimelineEntry[] = [
  {
    at: T,
    kind: "note",
    noteId: "n1",
    author: "rev-fixture",
    body: "Why?",
    system: false,
    root: true,
  },
  {
    at: T,
    kind: "commit",
    sha: "c2",
    title: "Commit c2",
    author: "alice-fixture",
    change: { kind: "renamed", from: "src/old.rs", to: "src/new.rs" },
  },
  { at: T, kind: "resolved", by: "alice-fixture" },
  { at: T, kind: "unresolved", by: "alice-fixture" },
];

const snapshotCode: CodeSnapshot = {
  sha: "c1",
  path: "src/cache.rs",
  firstLine: 1,
  lines: ["line 1"],
  anchorStart: 1,
  anchorEnd: 1,
  source: "file",
};

const history: ThreadHistory = {
  threadId: THREAD,
  reviewId: REVIEW,
  headSha: "c2",
  outdated: true,
  status: lineStatuses[0],
  touchedBy: ["c2"],
  thenSnapshot: snapshotCode,
  nowSnapshot: snapshotCode,
  diffLines: [
    {
      kind: "changed",
      then: { line: 1, text: "line 1", anchored: true },
      now: { line: 1, text: "line 1", anchored: true },
    },
  ],
  timeline: [timeline[0]],
};

const aiState: AiRunState = {
  reviewId: REVIEW,
  enabled: true,
  status: "done",
  queued: false,
  round: 2,
  adapter: "claude",
  error: "cancelled",
  startedAt: T,
  finishedAt: T,
  lastDoneRound: 2,
  droppedFindings: 0,
  droppedVerdicts: 0,
  droppedDuplicates: 0,
  skippedChecklist: 0,
  readOnlyGuaranteed: true,
  context: {
    rulesFiles: ["CLAUDE.md"],
    skill: { name: "code-review", source: "project" },
    worktree: true,
    language: { kind: "other", name: "Deutsch" },
    notes: ["Couldn't create a review worktree: …"],
  },
};

const reviewSkillList: ReviewSkillList = {
  skills: [
    {
      name: "fixture-tools:code-review",
      description: "Reviews code.",
      source: "plugin",
    },
  ],
  autoPick: "code-review",
  projectListed: true,
};

const reviewSkillChoices: ReviewSkillChoice[] = [
  { kind: "auto" },
  { kind: "none" },
  { kind: "skill", source: "user", name: "code-review" },
];

const finding: Finding = {
  id: FINDING,
  reviewId: REVIEW,
  round: 2,
  path: "src/cache.rs",
  line: 10,
  side: "new",
  severity: "high",
  title: "TTL of zero",
  body: "Every lookup misses.",
  dismissed: false,
  postedAt: T,
  postedUrl: "u",
  repeatOf: { reason: "agentMarked", threadId: THREAD, findingId: FINDING },
  relatedTo: [{ threadId: THREAD, findingId: FINDING }],
};

const checklist: ChecklistItem = {
  id: FINDING,
  reviewId: REVIEW,
  round: 2,
  text: "Is this intended?",
  checked: false,
  repeatOf: { reason: "similar", threadId: THREAD, findingId: FINDING },
  relatedTo: [{ threadId: THREAD, findingId: FINDING }],
};

const verdict: Verdict = {
  threadId: THREAD,
  reviewId: REVIEW,
  round: 2,
  aiLabel: "partly",
  aiReason: "Half of it.",
  userOverride: "addressed",
  label: "addressed",
};

const findingPosted: FindingPostResult = {
  findingId: FINDING,
  status: "posted",
  postedUrl: "u",
  postedAt: T,
};

const findingFailed: FindingPostResult = {
  findingId: FINDING,
  status: "failed",
  kind: "postRejected",
  message: "GitHub: refused",
};

const reviewRefresh: ReviewRefresh = { review: record, changed: true };

const localFile: LocalFile = {
  path: "src/cache.rs",
  absolutePath: "/Users/fixture/widgets/src/cache.rs",
  exists: true,
  cloneHead: "c".repeat(40),
  matchesReviewHead: false,
};

const cloneStatus: CloneStatus = {
  configured: true,
  path: "/Users/fixture/widgets",
  found: true,
  head: "c".repeat(40),
  matchesReviewHead: false,
};

const cloneSuggestion: CloneSuggestion = {
  repo: "github.com/acme-fixture/widgets",
  path: "/Users/fixture/code/widgets",
};

const repoSetup: RepoSetup = {
  key: "github.com/acme-fixture/widgets",
  host: "github.com",
  repo: "acme-fixture/widgets",
  provider: "github",
  openReviews: 2,
  clonePath: "/Users/fixture/code/widgets",
  clone: "found",
  reviewSkill: { kind: "skill", source: "user", name: "code-review" },
  reviewLanguage: { kind: "other", name: "Deutsch" },
};

const reviewLanguages: ReviewLanguage[] = [
  { kind: "auto" },
  { kind: "english" },
  { kind: "dutch" },
  { kind: "other", name: "Deutsch" },
];

const structs: Record<string, unknown> = {
  ReviewRecord: record,
  StoredThread: thread,
  ReviewFiles: files,
  CommentableLines: commentable,
  NewComment: newComment,
  "PostResult:posted": posted,
  "PostResult:failed": failed,
  PostedNote: postedNote,
  RefreshReport: report,
  ReviewRefresh: reviewRefresh,
  Settings: settings,
  CacheStats: cacheStats,
  Account: account,
  ThreadHistory: history,
  ...Object.fromEntries(lineStatuses.map((s) => [`LineStatus:${s.kind}`, s])),
  ...Object.fromEntries(timeline.map((e) => [`TimelineEntry:${e.kind}`, e])),
  AiRunState: aiState,
  Finding: finding,
  ChecklistItem: checklist,
  Verdict: verdict,
  "FindingPostResult:posted": findingPosted,
  "FindingPostResult:failed": findingFailed,
  LocalFile: localFile,
  CloneStatus: cloneStatus,
  CloneSuggestion: cloneSuggestion,
  ReviewSkillList: reviewSkillList,
  ...Object.fromEntries(
    reviewSkillChoices.map((c) => [`ReviewSkillChoice:${c.kind}`, c]),
  ),
  RepoSetup: repoSetup,
  ...Object.fromEntries(
    reviewLanguages.map((l) => [`ReviewLanguage:${l.kind}`, l]),
  ),
  ScriptResult: scriptResult,
  ScriptsDone: scriptsDone,
  GeneratedScript: generatedScript,
  ScriptDraft: scriptDraft,
  ScriptDraftProgress: draftProgress,
  ScriptDraftProgressEvent: draftProgressEvent,
  ScriptGenerationStatus: generationStatus,
};

const providerIds = [
  "github",
  "gitlab",
] as const satisfies readonly ProviderId[];
const reviewStates = [
  "open",
  "merged",
  "closed",
] as const satisfies readonly ReviewState[];
const reviewStatuses = [
  "needsReview",
  "needsReReview",
  "waitingOnAuthor",
  "closedWithoutApproval",
  "approved",
] as const satisfies readonly ReviewStatus[];
const roundReasons = [
  "newCommits",
  "authorReplied",
  "reRequested",
] as const satisfies readonly RoundReason[];
const sides = ["old", "new"] as const satisfies readonly Side[];
const accountStateKinds = [
  "loggedIn",
  "notLoggedIn",
  "error",
  "cliMissing",
] as const satisfies readonly AccountState["kind"][];
const fileStatuses = [
  "added",
  "modified",
  "removed",
  "renamed",
] as const satisfies readonly FileStatus[];
const postErrorKinds = [
  "notFound",
  "invalidInput",
  "githubUnavailable",
  "gitlabNotConfigured",
  "glabMissing",
  "gitlabAuth",
  "provider",
  "store",
  "io",
  "postRejected",
  "lineNotInDiff",
  "headMoved",
  "unsupported",
] as const satisfies readonly PostErrorKind[];
const outsideDiffs = [
  "none",
  "native",
  "fallback",
] as const satisfies readonly OutsideDiff[];
const myReviewStates = [
  "awaitingReview",
  "reviewStarted",
  "requestedChanges",
  "commented",
  "approved",
  "notReviewer",
] as const satisfies readonly MyReviewState[];
const reviewVerdicts = [
  "approve",
  "requestChanges",
  "comment",
] as const satisfies readonly ReviewVerdict[];
const skillSources = [
  "user",
  "plugin",
  "project",
] as const satisfies readonly SkillSource[];
const reviewSkillKinds = [
  "auto",
  "none",
  "skill",
] as const satisfies readonly ReviewSkillChoice["kind"][];
const reviewLanguageKinds = [
  "auto",
  "english",
  "dutch",
  "other",
] as const satisfies readonly ReviewLanguage["kind"][];
const cloneStates = [
  "notSet",
  "found",
  "missing",
  "notRepository",
] as const satisfies readonly CloneState[];
const lineStatusKinds = [
  "located",
  "deleted",
  "fileGone",
  "unlocated",
  "notCode",
] as const satisfies readonly LineStatus["kind"][];
const lineChangeKinds = [
  "modified",
  "deleted",
  "fileGone",
  "renamed",
] as const satisfies readonly LineChange["kind"][];
const timelineKinds = [
  "note",
  "commit",
  "resolved",
  "unresolved",
] as const satisfies readonly TimelineEvent["kind"][];
const snapshotSources = [
  "file",
  "diffHunk",
] as const satisfies readonly SnapshotSource[];
const diffKinds = [
  "same",
  "changed",
  "removed",
  "added",
] as const satisfies readonly DiffKind[];
const aiRunStatuses = [
  "notStarted",
  "running",
  "done",
  "failed",
] as const satisfies readonly AiRunStatus[];
const severities = [
  "high",
  "medium",
  "low",
] as const satisfies readonly Severity[];
const verdictLabels = [
  "addressed",
  "partly",
  "untouched",
] as const satisfies readonly VerdictLabel[];
const repeatReasons = [
  "agentMarked",
  "similar",
] as const satisfies readonly RepeatReason[];
const scriptStatuses = [
  "ok",
  "failed",
  "timedOut",
  "skipped",
  "notFound",
  "notExecutable",
] as const satisfies readonly ScriptStatus[];
const scriptDraftStatuses = [
  "idle",
  "running",
  "failed",
  "cancelled",
] as const satisfies readonly ScriptDraftStatus[];
const draftStages = [
  "reading",
  "starting",
  "thinking",
  "writing",
  "validating",
  "repairing",
] as const satisfies readonly DraftStage[];

// Each list names every member of its type (a compile error otherwise).
const covered: true[] = [
  true satisfies Covers<ProviderId, typeof providerIds>,
  true satisfies Covers<ReviewState, typeof reviewStates>,
  true satisfies Covers<ReviewStatus, typeof reviewStatuses>,
  true satisfies Covers<RoundReason, typeof roundReasons>,
  true satisfies Covers<Side, typeof sides>,
  true satisfies Covers<AccountState["kind"], typeof accountStateKinds>,
  true satisfies Covers<FileStatus, typeof fileStatuses>,
  true satisfies Covers<PostErrorKind, typeof postErrorKinds>,
  true satisfies Covers<OutsideDiff, typeof outsideDiffs>,
  true satisfies Covers<LineStatus["kind"], typeof lineStatusKinds>,
  true satisfies Covers<LineChange["kind"], typeof lineChangeKinds>,
  true satisfies Covers<TimelineEvent["kind"], typeof timelineKinds>,
  true satisfies Covers<SnapshotSource, typeof snapshotSources>,
  true satisfies Covers<DiffKind, typeof diffKinds>,
  true satisfies Covers<AiRunStatus, typeof aiRunStatuses>,
  true satisfies Covers<Severity, typeof severities>,
  true satisfies Covers<VerdictLabel, typeof verdictLabels>,
  true satisfies Covers<RepeatReason, typeof repeatReasons>,
  true satisfies Covers<ScriptStatus, typeof scriptStatuses>,
  true satisfies Covers<ScriptDraftStatus, typeof scriptDraftStatuses>,
  true satisfies Covers<DraftStage, typeof draftStages>,
  true satisfies Covers<MyReviewState, typeof myReviewStates>,
  true satisfies Covers<ReviewVerdict, typeof reviewVerdicts>,
  true satisfies Covers<SkillSource, typeof skillSources>,
  true satisfies Covers<ReviewSkillChoice["kind"], typeof reviewSkillKinds>,
  true satisfies Covers<ReviewLanguage["kind"], typeof reviewLanguageKinds>,
  true satisfies Covers<CloneState, typeof cloneStates>,
];

const enums: Record<string, readonly string[]> = {
  ProviderId: providerIds,
  ReviewState: reviewStates,
  ReviewStatus: reviewStatuses,
  RoundReason: roundReasons,
  Side: sides,
  "AccountState.kind": accountStateKinds,
  FileStatus: fileStatuses,
  PostErrorKind: postErrorKinds,
  OutsideDiff: outsideDiffs,
  "LineStatus.kind": lineStatusKinds,
  "LineChange.kind": lineChangeKinds,
  "TimelineEvent.kind": timelineKinds,
  SnapshotSource: snapshotSources,
  DiffKind: diffKinds,
  AiRunStatus: aiRunStatuses,
  Severity: severities,
  VerdictLabel: verdictLabels,
  RepeatReason: repeatReasons,
  MyReviewState: myReviewStates,
  ReviewVerdict: reviewVerdicts,
  SkillSource: skillSources,
  "ReviewSkillChoice.kind": reviewSkillKinds,
  "ReviewLanguage.kind": reviewLanguageKinds,
  CloneState: cloneStates,
  ScriptStatus: scriptStatuses,
  ScriptDraftStatus: scriptDraftStatuses,
  DraftStage: draftStages,
};

describe("wire shapes (types.ts vs the Rust snapshot)", () => {
  it("covers every struct and enum the snapshot records", () => {
    expect(Object.keys(structs).sort()).toEqual(
      Object.keys(snapshot.structs).sort(),
    );
    expect(Object.keys(enums).sort()).toEqual(
      Object.keys(snapshot.enums).sort(),
    );
    expect(covered).toHaveLength(Object.keys(enums).length);
  });

  it.each(Object.keys(snapshot.structs))("%s has the Rust keys", (name) => {
    const rust = (snapshot.structs as Record<string, string[]>)[name];
    expect(shape(structs[name])).toEqual(rust);
  });

  it.each(Object.keys(snapshot.enums))("%s has the Rust values", (name) => {
    const rust = (snapshot.enums as Record<string, string[]>)[name];
    expect([...enums[name]].sort()).toEqual([...rust].sort());
  });
});
