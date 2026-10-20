/**
 * Wire types for the Tauri IPC bridge.
 *
 * These mirror the serde shapes in `src-tauri` / `argus-core` exactly — change
 * them only in lockstep with the Rust side:
 * - Ids are plain UUID strings (`#[serde(transparent)]`).
 * - Structs and enums are camelCase (`rename_all = "camelCase"`).
 * - `Option<T>` is `T | null` (no field is skipped when empty).
 * - Times are RFC 3339 strings (`chrono::DateTime<Utc>`).
 */

export type ReviewId = string;
export type ThreadId = string;
/** RFC 3339 timestamp. */
export type Timestamp = string;

/** `provider::ProviderId`. */
export type ProviderId = "github" | "gitlab";

/** `provider::ReviewState`: the platform state. */
export type ReviewState = "open" | "merged" | "closed";

/** `review::ReviewStatus`: which list a review belongs in. */
export type ReviewStatus =
  | "needsReview"
  | "needsReReview"
  | "waitingOnAuthor"
  | "closedWithoutApproval"
  | "approved";

/** `review::RoundReason`: why a review needs the user again. */
export type RoundReason = "newCommits" | "authorReplied" | "reRequested";

/**
 * `store::ReviewRecord`, with `provider::ReviewRef` flattened in
 * (`provider` … `updatedAt`). `fingerprint` is never serialized.
 */
export interface ReviewRecord {
  id: ReviewId;
  provider: ProviderId;
  /** `github.com`, or the GitLab/GHE host (no scheme). */
  host: string;
  /** `owner/repo` on GitHub, the full project path on GitLab. */
  repo: string;
  /** PR number / MR iid. */
  number: number;
  url: string;
  title: string;
  /** Username of the MR/PR author. */
  author: string;
  updatedAt: Timestamp | null;
  state: ReviewState;
  draft: boolean;
  headSha: string;
  baseSha: string;
  lastReviewedSha: string | null;
  lastReviewedAt: Timestamp | null;
  approvedAt: Timestamp | null;
  /** 1 for the first round, +1 each time it needs re-review. */
  round: number;
  /** Non-empty exactly when `status` is `needsReReview`. */
  roundReasons: RoundReason[];
  status: ReviewStatus;
  /** The user's open threads nobody touched since. */
  untouchedThreads: number;
  requested: boolean;
  rerequestedAt: Timestamp | null;
  listed: boolean;
  /** The user's username on this review's platform. */
  viewer: string;
  fetchedAt: Timestamp;
}

/** `provider::Side`. */
export type Side = "old" | "new";

/** `provider::Anchor`: where a thread was left, as of its first comment. */
export interface Anchor {
  pathOld: string | null;
  pathNew: string | null;
  lineOld: number | null;
  lineNew: number | null;
  /** First line of a multi-line comment, on `side`. */
  startLine: number | null;
  side: Side;
  baseSha: string | null;
  startSha: string | null;
  headSha: string;
  diffHunk: string | null;
  /**
   * GitHub: the line in the current diff on `side` (`null` once outdated;
   * `lineOld`/`lineNew` are the original line). Always `null` on GitLab.
   */
  currentLine: number | null;
}

/** `provider::Note`: one comment in a thread. */
export interface Note {
  /** The platform's note/comment id. */
  id: string;
  author: string;
  body: string;
  createdAt: Timestamp;
  /** A platform-generated note (GitLab system notes). */
  system: boolean;
}

/**
 * `store::StoredThread`, with `provider::Thread` flattened in (`id` …
 * `notes`). `id` is the platform's thread id; `threadId` is Argus' own.
 */
export interface StoredThread {
  threadId: ThreadId;
  reviewId: ReviewId;
  /** The user's thread: started by them or with a comment of theirs in it. */
  mine: boolean;
  /** Started by the user (implies `mine`). */
  startedByMe: boolean;
  id: string;
  author: string;
  createdAt: Timestamp;
  resolved: boolean;
  outdated: boolean;
  /** `null` for a discussion on the MR/PR as a whole. */
  anchor: Anchor | null;
  /** Oldest first. */
  notes: Note[];
}

/** `provider::FileStatus`. */
export type FileStatus = "added" | "modified" | "removed" | "renamed";

/**
 * `files::FileEntry`: one changed file, with `provider::ChangedFile`
 * flattened in (`path` … `tooLarge`).
 */
export interface FileEntry {
  /** The path at head (a removed file's former path). */
  path: string;
  /** The path before a rename; `null` unless `renamed`. */
  oldPath: string | null;
  status: FileStatus;
  /** 0 when the platform sent no patch. */
  additions: number;
  deletions: number;
  /** Unified patch hunks, when the platform sent them. */
  patch: string | null;
  /** The file's content at head (GitHub blob sha, GitLab diff digest). */
  contentSha: string;
  binary: boolean;
  /** The platform left the patch out as too large. */
  tooLarge: boolean;
  /** Marked viewed and unchanged at head since. */
  viewed: boolean;
}

/** `files::ReviewFiles`: what `list_files` returns. */
export interface ReviewFiles {
  reviewId: ReviewId;
  /** The left side of every file (merge base / GitLab base_sha / watermark). */
  baseSha: string;
  headSha: string;
  /** The diff runs from the last-reviewed commit to head. */
  sinceLastReview: boolean;
  /** The platform's file limit was hit; more files changed. */
  truncated: boolean;
  /**
   * `baseSha` only stands in for the left side (GitHub could not compute a
   * large PR's merge base, and no local clone could): removed lines may not
   * line up, so they take no comments.
   */
  baseApproximate: boolean;
  /**
   * The left side of the review's own diff (the local clone's merge base
   * when it replaced a stand-in): what `commentable_lines` reports, since
   * comments always go on that diff. Equals `baseSha` unless
   * `sinceLastReview`, where `baseSha` is the compare's left side.
   */
  reviewBaseSha: string;
  files: FileEntry[];
}

/**
 * `post::NewComment`: a new line comment as the user wrote it. The optional
 * fields may be left out (serde `default`).
 */
export interface NewComment {
  /** The file's path at head. */
  path: string;
  /** The (last) line, on `side`. */
  line: number;
  side: Side;
  /** First line of a multi-line comment, same side and hunk. */
  startLine?: number | null;
  body: string;
  /** The head the lines were read at; `null` = the stored head. */
  headSha?: string | null;
  /** The diff's left side the old-side lines were read at. */
  baseSha?: string | null;
}

/** `post::PostedComment`: a comment the platform accepted. */
export interface PostedComment {
  url: string;
  /** As in `StoredThread.id`. */
  providerThreadId: string;
  /** As in `Note.id`. */
  providerNoteId: string;
  /** Where it landed, after the move to the current head. */
  path: string;
  side: Side;
  line: number;
  startLine: number | null;
  headSha: string;
}

/** `post::PostErrorKind`, plus the IPC-only kinds `IpcError` can carry. */
export type PostErrorKind =
  | "notFound"
  | "invalidInput"
  | "githubUnavailable"
  | "gitlabNotConfigured"
  | "gitlabAuth"
  | "provider"
  | "store"
  | "io"
  /** The platform refused the write; the message carries its reason. */
  | "postRejected"
  /** The line is not (any more) in the current diff. */
  | "lineNotInDiff"
  /** The review got new commits between reading and writing. */
  | "headMoved";

/** `post::PostError`: `{ kind, message }`, the shape of `IpcError`. */
export interface PostError {
  kind: PostErrorKind;
  message: string;
}

/** `post::PostResult`: one comment of a batch, in the batch's order. */
export type PostResult =
  ({ status: "posted" } & PostedComment) | ({ status: "failed" } & PostError);

/** `post::PostedNote`: a reply the platform accepted. */
export interface PostedNote {
  url: string;
  providerThreadId: string;
  providerNoteId: string;
}

/** `post::CommentableHunk`: one hunk's commentable lines, ascending. */
export interface CommentableHunk {
  /** Old-side lines (removed and context rows). */
  old: number[];
  /** New-side lines (added and context rows). */
  new: number[];
}

/**
 * `post::CommentableLines`: the lines of one file a new comment can go on.
 * A range must stay inside one hunk.
 */
export interface CommentableLines {
  reviewId: ReviewId;
  path: string;
  baseSha: string;
  headSha: string;
  /** Empty for a file without a text patch (binary, too large). */
  hunks: CommentableHunk[];
}

/** `settings::GithubSettings`. */
export interface GithubSettings {
  enabled: boolean;
  host: string;
  /** A logged-in `gh` account; `null` is gh's active account. */
  account: string | null;
}

/** `settings::GitlabSettings`. Empty `baseUrl` = GitLab off. */
export interface GitlabSettings {
  baseUrl: string;
}

/** `settings::AiSettings`. */
export interface AiSettings {
  enabled: boolean;
  adapter: string;
  args: string[];
  /**
   * The agent binary's path; `null` (or blank) finds the adapter's CLI
   * through the login shell. Always sent back by the core; optional here so
   * a settings object without it still saves (serde `default`).
   */
  binary?: string | null;
}

/** `settings::Settings` (settings.json). Never holds a secret. */
export interface Settings {
  github: GithubSettings;
  gitlab: GitlabSettings;
  pollIntervalSecs: number;
  ai: AiSettings;
  /** Local clone per repository, keyed `host/repo`. */
  clonePaths: Record<string, string>;
}

/** `poll::RefreshError`: one failure during a refresh. */
export interface RefreshError {
  provider: ProviderId;
  /** `null` when the provider as a whole failed. */
  review: ReviewId | null;
  message: string;
  /** The provider cannot be used right now. */
  unavailable: boolean;
  /** RFC 3339; set when the platform rate-limited the provider. */
  rateLimitedUntil: Timestamp | null;
}

/** `poll::RefreshReport`: what one refresh did. */
export interface RefreshReport {
  changed: ReviewId[];
  errors: RefreshError[];
  refreshedAt: Timestamp;
}

/** `provider::GithubStatus`. */
export interface GithubStatus {
  enabled: boolean;
  host: string;
  installed: boolean;
  loggedIn: boolean;
}

/** `provider::GitlabProblem`: why a configured GitLab is off anyway. */
export type GitlabProblem = "invalidUrl";

/** `provider::GitlabStatus`. */
export interface GitlabStatus {
  configured: boolean;
  baseUrl: string;
  tokenPresent: boolean;
  /** `null` when nothing is wrong. */
  problem: GitlabProblem | null;
  /** Plain http to a host other than this machine: the token is unencrypted. */
  insecure: boolean;
}

/** `provider::ProviderStatus`. */
export interface ProviderStatus {
  github: GithubStatus;
  gitlab: GitlabStatus;
}

/** Payload of the `review:changed` event (`events::ReviewRefPayload`). */
export interface ReviewChangedEvent {
  reviewId: ReviewId;
}

// --- Thread history (M3) ---
// `history::*` (crates/argus-core/src/history/mod.rs).

/**
 * `history::LineStatus`: where the anchored lines are at head. Lines are
 * 1-based; `removedBy` is a commit sha, `null` when no single commit removed
 * them (the review's own diff, a rebase).
 */
export type LineStatus =
  | {
      kind: "located";
      path: string;
      startLine: number;
      line: number;
      /** The path or the line numbers differ from then. */
      moved: boolean;
      /** The file was renamed on the way (`path` is the new name). */
      renamed: boolean;
      /** A commit changed the lines (see `touchedBy`). */
      touched: boolean;
    }
  | { kind: "deleted"; path: string; removedBy: string | null }
  | { kind: "fileGone"; path: string; removedBy: string | null }
  /** Could not be followed reliably; `reason` is Argus-owned text. */
  | { kind: "unlocated"; reason: string }
  /** A discussion on the review as a whole: no code, no snapshots. */
  | { kind: "notCode" };

/** `history::SnapshotSource`: the file, or GitHub's `diff_hunk` fallback. */
export type SnapshotSource = "file" | "diffHunk";

/** `history::CodeSnapshot`: a window of code around the anchored lines. */
export interface CodeSnapshot {
  sha: string;
  path: string;
  /** 1-based number of `lines[0]`. */
  firstLine: number;
  lines: string[];
  /** The anchored range in file line numbers; `null`: a deleted line's surroundings. */
  anchorStart: number | null;
  anchorEnd: number | null;
  source: SnapshotSource;
}

/** `history::DiffKind`. */
export type DiffKind = "same" | "changed" | "removed" | "added";

/** `history::DiffCell`. */
export interface DiffCell {
  /** The file line number. */
  line: number;
  text: string;
  /** Inside the anchored range of its side. */
  anchored: boolean;
}

/** `history::DiffRow`: one row of the then/now diff; either side may be empty. */
export interface DiffRow {
  kind: DiffKind;
  then: DiffCell | null;
  now: DiffCell | null;
}

/** `history::LineChange`: what one commit did to the anchored lines. */
export type LineChange =
  | { kind: "modified" }
  | { kind: "deleted" }
  | { kind: "fileGone" }
  | { kind: "renamed"; from: string; to: string };

/** `history::TimelineEvent`, flattened into `TimelineEntry`. */
export type TimelineEvent =
  | {
      kind: "note";
      noteId: string;
      author: string;
      body: string;
      system: boolean;
      /** The thread's first comment. */
      root: boolean;
    }
  | {
      kind: "commit";
      sha: string;
      title: string;
      author: string;
      change: LineChange;
    }
  | { kind: "resolved"; by: string }
  | { kind: "unresolved"; by: string };

/**
 * `history::TimelineEntry`. `at` is `null` only for a commit the platform
 * sent no date for (sorted last).
 */
export type TimelineEntry = { at: Timestamp | null } & TimelineEvent;

/** `history::ThreadHistory`: what `thread_history` returns. */
export interface ThreadHistory {
  threadId: ThreadId;
  reviewId: ReviewId;
  /** The head this was computed at. */
  headSha: string;
  /** The platform's own outdated flag. */
  outdated: boolean;
  status: LineStatus;
  /** Commit shas that changed the lines, oldest first. */
  touchedBy: string[];
  thenSnapshot: CodeSnapshot | null;
  nowSnapshot: CodeSnapshot | null;
  /** Empty unless both snapshots exist. */
  diffLines: DiffRow[];
  /** Oldest first. */
  timeline: TimelineEntry[];
}

// --- AI pass (M5) ---
// `ai::model::*` (crates/argus-core/src/ai/model.rs).

export type FindingId = string;
export type ChecklistId = string;

/** `ai::AiRunStatus`. */
export type AiRunStatus = "notStarted" | "running" | "done" | "failed";

/**
 * `ai::AiRunState`: what `ai_state` returns and `ai:state` carries — the
 * review's latest run plus whether another waits for a slot.
 */
export interface AiRunState {
  reviewId: ReviewId;
  /** The AI pass is on in settings; when off, `status` is `notStarted`. */
  enabled: boolean;
  status: AiRunStatus;
  /** A run waits for a free slot (runtime only). */
  queued: boolean;
  /** The round the latest run was for. */
  round: number | null;
  /** The adapter id the latest run used (`claude`, `auggie`). */
  adapter: string | null;
  /** Why the latest run failed (app-owned; `interrupted`, `cancelled`). */
  error: string | null;
  startedAt: Timestamp | null;
  finishedAt: Timestamp | null;
  /** The round of the last run that succeeded. */
  lastDoneRound: number | null;
  /** Findings the latest successful run dropped (path not in the diff). */
  droppedFindings: number;
  /** Verdicts the latest successful run dropped (unknown thread ids). */
  droppedVerdicts: number;
  /**
   * Findings the latest successful run set aside because the review already
   * makes their point (an existing thread, anyone's, or a posted finding);
   * listed by `list_skipped_findings`, never shown as findings.
   */
  droppedDuplicates: number;
  /** Checklist items the latest successful run left out for that reason. */
  skippedChecklist: number;
  /**
   * Whether Argus can guarantee the configured agent only reads. `false` for
   * `auggie`, whose user-configured MCP tools cannot be turned off.
   */
  readOnlyGuaranteed: boolean;
}

/** `ai::Severity`. */
export type Severity = "high" | "medium" | "low";

/** `ai::Finding`: one problem, anchored to a line of the diff. */
export interface Finding {
  id: FindingId;
  reviewId: ReviewId;
  round: number;
  path: string;
  line: number;
  side: Side;
  severity: Severity;
  title: string;
  body: string;
  dismissed: boolean;
  /** Set once the user posted it (`post_findings`). */
  postedAt: Timestamp | null;
  postedUrl: string | null;
  /**
   * What the review already says that this repeats: set on a finding set
   * aside as already raised, and kept once the user shows it anyway.
   */
  repeatOf: RepeatOf | null;
  /**
   * The points of the review the agent said this overlaps, when it does not
   * repeat them ("related to bob's comment"). A high-severity finding is
   * never set aside on the agent's mark alone: it lists them here.
   */
  relatedTo: RelatedRef[];
}

/** `ai::RelatedRef`: a thread of the review, or a posted finding. */
export interface RelatedRef {
  threadId: ThreadId | null;
  findingId: FindingId | null;
}

/**
 * `ai::RepeatReason`: the agent listed the thread as related and the text
 * still says what it says, or Argus' own comparison found it.
 */
export type RepeatReason = "agentMarked" | "similar";

/** `ai::RepeatOf`: a thread of the review, or a posted finding. */
export interface RepeatOf {
  reason: RepeatReason;
  threadId: ThreadId | null;
  findingId: FindingId | null;
}

/** `ai::ChecklistItem`: "look here yourself". */
export interface ChecklistItem {
  id: ChecklistId;
  reviewId: ReviewId;
  round: number;
  text: string;
  checked: boolean;
  /**
   * What the review already says that this repeats: set on an item set
   * aside as already raised, and kept once the user shows it anyway.
   */
  repeatOf: RepeatOf | null;
}

/** `ai::FindingPost`: a finding to post, with the user's edit of its text. */
export interface FindingPost {
  findingId: FindingId;
  /** Left out or `null`: `**title**` and the finding's body. */
  body?: string | null;
}

/** `ai::FindingPostResult`: one finding of a batch, in the batch's order. */
export type FindingPostResult = { findingId: FindingId } & (
  | { status: "posted"; postedUrl: string; postedAt: Timestamp }
  | ({ status: "failed" } & PostError)
);

/** `ai::VerdictLabel`. */
export type VerdictLabel = "addressed" | "partly" | "untouched";

/**
 * `ai::Verdict`: the AI's hint and the user's own call on one thread, kept
 * apart. Never written to the platform.
 */
export interface Verdict {
  threadId: ThreadId;
  reviewId: ReviewId;
  round: number;
  aiLabel: VerdictLabel | null;
  aiReason: string | null;
  userOverride: VerdictLabel | null;
  /** What to show: the override when set, else the AI's label. */
  label: VerdictLabel | null;
}

/** Structured error every IPC command can reject with (`IpcError`). */
export interface IpcError {
  /** Human-readable, sanitized description. */
  message: string;
  /** Stable machine-readable category, e.g. `"invalidInput"`. */
  kind: string;
}
