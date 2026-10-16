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
  /** Started by the current user. */
  mine: boolean;
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

/** `provider::GitlabStatus`. */
export interface GitlabStatus {
  configured: boolean;
  baseUrl: string;
  tokenPresent: boolean;
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

/** Structured error every IPC command can reject with (`IpcError`). */
export interface IpcError {
  /** Human-readable, sanitized description. */
  message: string;
  /** Stable machine-readable category, e.g. `"invalidInput"`. */
  kind: string;
}
