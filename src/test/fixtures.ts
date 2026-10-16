/** Fictitious wire objects for tests. */

import type {
  FileEntry,
  ProviderStatus,
  ReviewFiles,
  ReviewRecord,
  Settings,
  StoredThread,
} from "../ipc/types";

let seq = 0;
const uuid = (n: number) =>
  `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;

export function review(overrides: Partial<ReviewRecord> = {}): ReviewRecord {
  seq += 1;
  return {
    id: uuid(seq),
    provider: "github",
    host: "github.com",
    repo: "acme-fixture/widgets",
    number: seq,
    url: `https://github.com/acme-fixture/widgets/pull/${seq}`,
    title: `Fixture change ${seq}`,
    author: "alice-fixture",
    updatedAt: "2026-10-16T10:00:00Z",
    state: "open",
    draft: false,
    headSha: "a".repeat(40),
    baseSha: "b".repeat(40),
    lastReviewedSha: null,
    lastReviewedAt: null,
    approvedAt: null,
    round: 1,
    roundReasons: [],
    status: "needsReview",
    untouchedThreads: 0,
    requested: true,
    rerequestedAt: null,
    listed: true,
    viewer: "rev-fixture",
    fetchedAt: "2026-10-16T10:05:00Z",
    ...overrides,
  };
}

export function thread(overrides: Partial<StoredThread> = {}): StoredThread {
  seq += 1;
  return {
    threadId: uuid(1000 + seq),
    reviewId: uuid(1),
    mine: false,
    id: String(seq),
    author: "bob-fixture",
    createdAt: "2026-10-15T09:00:00Z",
    resolved: false,
    outdated: false,
    anchor: {
      pathOld: "src/cache.rs",
      pathNew: "src/cache.rs",
      lineOld: null,
      lineNew: 42,
      startLine: null,
      side: "new",
      baseSha: null,
      startSha: null,
      headSha: "c".repeat(40),
      diffHunk: null,
      currentLine: null,
    },
    notes: [
      {
        id: `n${seq}`,
        author: overrides.author ?? "bob-fixture",
        body: `Note body ${seq}`,
        createdAt: "2026-10-15T09:00:00Z",
        system: false,
      },
    ],
    ...overrides,
  };
}

export function file(overrides: Partial<FileEntry> = {}): FileEntry {
  return {
    path: "src/cache.rs",
    oldPath: null,
    status: "modified",
    additions: 2,
    deletions: 1,
    patch: "@@ -1 +1,2 @@\n-old\n+new\n+more",
    contentSha: `sha-${overrides.path ?? "src/cache.rs"}`,
    binary: false,
    tooLarge: false,
    viewed: false,
    ...overrides,
  };
}

export function reviewFiles(overrides: Partial<ReviewFiles> = {}): ReviewFiles {
  return {
    reviewId: uuid(1),
    baseSha: "b".repeat(40),
    headSha: "a".repeat(40),
    sinceLastReview: false,
    truncated: false,
    files: [file()],
    ...overrides,
  };
}

export function settings(overrides: Partial<Settings> = {}): Settings {
  return {
    github: { enabled: true, host: "github.com", account: null },
    gitlab: { baseUrl: "" },
    pollIntervalSecs: 180,
    ai: { enabled: true, adapter: "claude", args: [], binary: null },
    clonePaths: {},
    ...overrides,
  };
}

export function status(
  overrides: Partial<ProviderStatus> = {},
): ProviderStatus {
  return {
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
    },
    ...overrides,
  };
}

/**
 * A fake `listen`: records handlers by event name so a test can fire
 * backend events at the stores.
 */
export function fakeEvents() {
  const handlers = new Map<string, ((e: { payload: unknown }) => void)[]>();
  return {
    listen: (name: string, cb: (e: { payload: unknown }) => void) => {
      handlers.set(name, [...(handlers.get(name) ?? []), cb]);
      return Promise.resolve(() => {
        handlers.set(
          name,
          (handlers.get(name) ?? []).filter((h) => h !== cb),
        );
      });
    },
    emit: (name: string, payload: unknown) => {
      for (const h of handlers.get(name) ?? []) h({ payload });
    },
    count: (name: string) => handlers.get(name)?.length ?? 0,
  };
}
