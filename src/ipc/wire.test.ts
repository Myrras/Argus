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
  AiRunState,
  AiRunStatus,
  ChecklistItem,
  CodeSnapshot,
  CommentableLines,
  DiffKind,
  FileStatus,
  Finding,
  GitlabProblem,
  FindingPostResult,
  LineChange,
  LineStatus,
  LocalFile,
  NewComment,
  PostedNote,
  PostErrorKind,
  PostResult,
  ProviderId,
  ProviderStatus,
  RefreshReport,
  ReviewRefresh,
  RepeatReason,
  ReviewFiles,
  ReviewRecord,
  ReviewVerdict,
  MyReviewState,
  ReviewState,
  ReviewStatus,
  RoundReason,
  Settings,
  Severity,
  Side,
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
      review: REVIEW,
      message: "GitLab is not set up.",
      unavailable: true,
      rateLimitedUntil: T,
    },
  ],
  refreshedAt: T,
};

const settings: Settings = {
  github: { enabled: true, host: "github.com", account: "work" },
  gitlab: { baseUrl: "" },
  pollIntervalSecs: 180,
  ai: { enabled: true, adapter: "claude", args: [], binary: "/bin/claude" },
  clonePaths: {},
  editorOverrides: {},
  defaultEditor: null,
};

const status: ProviderStatus = {
  github: {
    enabled: true,
    host: "github.com",
    installed: true,
    loggedIn: true,
  },
  gitlab: {
    configured: true,
    baseUrl: "https://gitlab.example.test",
    tokenPresent: true,
    problem: "invalidUrl",
    insecure: true,
  },
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
};

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
  ProviderStatus: status,
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
const gitlabProblems = [
  "invalidUrl",
] as const satisfies readonly GitlabProblem[];
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
  "gitlabAuth",
  "provider",
  "store",
  "io",
  "postRejected",
  "lineNotInDiff",
  "headMoved",
  "unsupported",
] as const satisfies readonly PostErrorKind[];
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

// Each list names every member of its type (a compile error otherwise).
const covered: true[] = [
  true satisfies Covers<ProviderId, typeof providerIds>,
  true satisfies Covers<ReviewState, typeof reviewStates>,
  true satisfies Covers<ReviewStatus, typeof reviewStatuses>,
  true satisfies Covers<RoundReason, typeof roundReasons>,
  true satisfies Covers<Side, typeof sides>,
  true satisfies Covers<GitlabProblem, typeof gitlabProblems>,
  true satisfies Covers<FileStatus, typeof fileStatuses>,
  true satisfies Covers<PostErrorKind, typeof postErrorKinds>,
  true satisfies Covers<LineStatus["kind"], typeof lineStatusKinds>,
  true satisfies Covers<LineChange["kind"], typeof lineChangeKinds>,
  true satisfies Covers<TimelineEvent["kind"], typeof timelineKinds>,
  true satisfies Covers<SnapshotSource, typeof snapshotSources>,
  true satisfies Covers<DiffKind, typeof diffKinds>,
  true satisfies Covers<AiRunStatus, typeof aiRunStatuses>,
  true satisfies Covers<Severity, typeof severities>,
  true satisfies Covers<VerdictLabel, typeof verdictLabels>,
  true satisfies Covers<RepeatReason, typeof repeatReasons>,
  true satisfies Covers<MyReviewState, typeof myReviewStates>,
  true satisfies Covers<ReviewVerdict, typeof reviewVerdicts>,
];

const enums: Record<string, readonly string[]> = {
  ProviderId: providerIds,
  ReviewState: reviewStates,
  ReviewStatus: reviewStatuses,
  RoundReason: roundReasons,
  Side: sides,
  GitlabProblem: gitlabProblems,
  FileStatus: fileStatuses,
  PostErrorKind: postErrorKinds,
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
