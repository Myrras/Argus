# CLAUDE.md — Argus

Guidance for AI agents (and humans) working in this repository.

## What Argus is

Argus is a desktop **review cockpit for GitLab merge requests and GitHub pull
requests**, built in **Rust + Tauri 2 + React + TypeScript + Vite**.
macOS-first; Windows kept in mind.

## Boilerplate (shared skeleton)

The skeleton — Cargo workspace shape, `justfile`, `scripts/`, CI, shared config
files, design tokens, layering/IPC conventions — comes from
<https://github.com/RobbieMinderhoud/rust-application-boilerplate>
(`../rust-application-boilerplate` locally). CI/release setup follows the
sibling app Iris (`../Iris`).

- **Rule:** a change to the **shared skeleton or conventions** (not Argus
  domain code) also goes as a **PR against the boilerplate**.

## Status

- ✅ **Milestone 1 — Skeleton.** Renamed boilerplate, CI + release pipeline.
- ✅ **Milestone 2 — Providers and the review list.** GitHub (`gh`) and GitLab
  (REST, keychain token) providers, SQLite store, settings, round detection,
  background polling; IPC commands + events; the four-tab list, detail view
  with threads (mine first, outdated never hidden), settings, provider
  status. The boilerplate greet/notes sample is gone.
  - Island shell, ported from Iris (Podium is the UI reference): same tokens
    (Argus violet is the only brand difference), title bar on the canvas
    (`titleBarStyle: Overlay` on macOS, own `WindowControls` on Windows),
    sidebar island (`Sidebar`: Reviews = `ReviewList`, Providers =
    `ProvidersSection`), resizer, the work column (the detail island;
    `WelcomeScreen` without a selection), `Modal`, `Toasts` (`toastStore`), `ContextMenu`, lucide
    `icons.tsx`. UI prefs are localStorage-only: `themeStore` → `argus.theme`
    (dark/light/retro), `layoutStore` → `argus.layout`.
  - Settings as in Iris: `SettingsModal` (tabs General / GitHub / GitLab /
    AI / Shortcuts; switches and steppers apply at once, text fields in a
    form with its own Save, failures as toasts), `settingsStore` (UI prefs →
    `argus.settings`: reduced motion, shortcut overrides),
    `backendSettingsStore` (the core's `settings.json`, provider status,
    token presence; the GitLab token only ever goes to `set_gitlab_token`),
    `settingsRequestStore` (open on a tab from deeper in the tree). Opened by
    the macOS menu's Settings… (`menu:open-settings`, `lib.rs`) or the
    keymap's `openSettings` (Ctrl+, elsewhere). Shortcuts: `lib/keymap`
    (rebindable, bare keys allowed unlike Iris), `useAppShortcuts`,
    `ShortcutsTab`. Local clones use the dialog plugin's folder picker.
  - Review view (one page, no tabs): `ReviewDetail` = header + `ReviewView`
    = `FileTree` (`data-slot="file-tree"`, a Conversation entry first, then
    tree/flat + filter) beside the page (`data-slot="diff"`):
    `ConversationCard` (threads without a code reference, mine first; plus
    "On other files" for anchored threads whose file the list does not show,
    `diffThreads.conversationThreads`) then one `FileDiffCard` per file,
    contents loaded lazily (IntersectionObserver or selection; cached per
    review/sha/path in `codeViewStore`), binary and large (>1500 lines or no
    patch) diffs as cards. `codeViewStore.selected`/`order` hold the
    `CONVERSATION` key first, so j/k stop there before the files.
    `DiffEditor` wraps `@codemirror/merge` (`unifiedMergeView` /
    `MergeView`, layout in `layoutStore.diffLayout`; the old `detailTab` key
    is ignored); the editor setup (`lib/codemirror`) is Selene's, themed on
    the tokens (`--diff-*`). Threads: `lib/diffThreads` places current ones
    in CodeMirror block widgets (`ThreadBlock`, its own React root), outdated
    ones are listed at the top of their file (GitHub `diff_hunk` / GitLab
    line + commit). `ThreadBlock` is the one thread card everywhere; its
    "Then/now history" toggle expands the M3 slot (`data-slot="then-now"`)
    in place. Comment bodies: `Markdown` (Podium's react-markdown +
    remark-gfm, no raw HTML; links via the opener plugin; images as links;
    fences highlighted by `CodeEditorBlock` with `languageForInfo`, jq as a
    stream mode). Suggestions: `lib/suggestions` (pure: GitHub
    `suggestion` = start_line..line, GitLab `suggestion:-N+M`; original
    lines from `file_contents` at the anchor's commit, else `diff_hunk`)
    rendered by `SuggestedChange`; never applied. Core: `fetch_files`
    (`DiffRange::Review` / `Compare`) per provider; left side = GitHub merge
    base (compare API), GitLab latest version `base_sha`; since-last-review
    = GitHub `compare/last...head` (left: its merge base), GitLab
    `compare?straight=true` (left: last). Viewed marks: `viewed_files`
    (migration 2), keyed path + `content_sha` (GitHub blob sha, GitLab diff
    digest) of the review's own diff. Keys `nextFile`/`prevFile`/
    `toggleViewed` (scope `files`).
- ✅ **Milestone 4 — Posting.** Core `post/`: `post_comments` (one by one,
  a result per comment in order, moved to the platform's current head),
  `reply_to_thread`, `approve_review` (pinned to the head the user saw:
  GitHub review `commit_id`, GitLab approve `sha`; `headMoved` when the
  stored or platform head differs) and `commentable_lines` (per hunk;
  cached per review + stored head). `PostError { kind, message }` → `IpcError`
  by `kind.as_str()`; new kinds `postRejected`, `lineNotInDiff`,
  `headMoved`. Shell `commands/post.rs` emits `review:changed` after every
  successful write. Only these four commands write, each from a button or
  shortcut handler; nothing auto-posts.
  - UI: `DiffEditor` takes `items` (blocks under lines besides threads) and
    `comment` (`CommentGutter`: "+" on commentable lines per side, click /
    shift-click / drag, selection + focused-line decorations); the unified
    view comments on new-side lines only. `FileDiffCard` loads the lines
    (`commentStore.loadCommentable`) and places the composer and drafts.
    `commentStore`: drafts per review (localStorage `argus.drafts.v1.<id>`),
    the one composer, reply composers, the focused line; `CommentComposer`
    (textarea + Preview via `Markdown`) and `CommentBlocks`
    (`InlineComposer`, `DraftBlock`, `ReplyComposer`). Header: pending
    count + Post all, Approve… with `ApproveModal`. Keys `commentLine`
    (`files`), `submitComment` / `cancelComment` (new `composer` scope;
    Escape is `fixed`). Ranges: `lib/commentRanges` (one hunk).
- ✅ **Milestone 3 — Thread history.** Core `history/`:
  `thread_history(ThreadId)` / `review_histories(ReviewId)` (then/now
  snapshots, line walk to head, diff rows, timeline of notes, commits and
  resolve events; cached per thread + head in SQLite), `cached_snapshot`
  (for the AI prompt), `thread_review`. Provider `commit_diff` +
  `thread_resolutions`. A history call that computes walks rewrites the
  review's untouched count. Shell `commands/history.rs` (emits
  `review:changed` when the untouched count moved).
  - UI: `historyStore` (histories per review + head, re-read on
    `review:changed` and when the thread store sees changed content; the
    then/now `expanded` set and the `focused` thread for the key),
    `useThreadHistory` (per-thread load/error for `ThreadBlock`),
    `ThreadHistory` = `ThenNow` (Then/Now columns aligned from the backend's
    diff rows, side by side or stacked, a status line per `LineStatus`) +
    `Timeline` (notes, commits, resolves; long comments fold),
    `ThenNowEditor` (lazy CodeMirror column, plain rows until loaded),
    pure rules in `lib/threadHistory`. Your own threads open it by default;
    key `toggleThenNow` (`h`, scope `files`) acts on the focused thread (M6).
- ✅ **Milestone 5 — The AI pass.** Core `ai/`: a local agent CLI (`claude`,
  `auggie`; read-only flags, token-free env) reviews the diff headless:
  findings, a checklist, and verdicts on the user's threads in later rounds.
  Per review serial, two runs at once. `run_ai` / `cancel_ai`,
  `on_refresh_report` (the poll hook: queues runs, never waits),
  `on_ai_state`, `dismiss_finding`, `set_checklist_checked`,
  `set_verdict_override` (local only), `post_findings` (through the post
  API; explicit only). Migrations 3 and 4 are intentional no-ops, 5 adds
  `ai_runs` + `findings.side`; `AiSettings.binary` (optional agent path).
  Shell `commands/ai.rs` (`post_findings` emits `review:changed` after any
  posted). TS types/wrappers in the `// --- Thread history (M3) ---` and
  `// --- AI pass (M5) ---` sections of `src/ipc/*.ts`.
  - UI: `aiStore` (latest run per review via `ai_state` + `ai:state`;
    findings, checklist and verdicts for opened reviews; per finding the
    selection, edited text, open editor and last post failure; with the pass
    off nothing shows), `AiStatus` (header chip, Run again / Retry / Cancel,
    panel toggle; `AiRunIndicator` in the list), `AiPanel` (the island right
    of the detail in `App`'s `workRow`: `FindingsList` by severity with
    "Post selected", over `Checklist`; open state in
    `layoutStore.aiPanelOpen`), `FindingBlock` (inline under its line or at
    the top of its file), `VerdictBadge` (in the thread header; your own call
    overrides, Argus-only), pure rules in `lib/findings`. Keys
    `toggleFindings` (`a`, scope `files`) and `toggleFindingSelected` (`x`).
    Only the "Post selected" button posts.
- ✅ **Milestone 6 — Local clone.** Core `git/`: with a clone configured per
  repository (`Settings::clone_paths`), the history walk and `Core::file_at`
  read through plain `git` (`rev-list`, `diff-tree`, `cat-file --batch`)
  instead of the API: faster, exact, never truncated. Read-only by
  construction: object reads only (no checkout, fetch or status),
  `GIT_OPTIONAL_LOCKS=0`, no lazy fetch of a partial clone (git >= 2.44),
  no shell, 30 s timeout, an env without tokens or `GIT_*` redirections.
  Always a fallback, never a requirement: an unfetched commit, shallow
  boundary, type change, non-UTF-8 file or any git failure is answered by the
  API, per thread or per file (`git::file_or`; `compute_histories` takes
  `clone: Option<&Path>`). Argus never changes or fetches the clone.
- ✅ **Milestone 6 — Provider polish.** Polling is polite. `provider/conditional`:
  in-memory ETag / Last-Modified cache per provider (LRU, bounded), so
  unchanged requests are a 304 (free on GitHub). GitHub: `gh api -i` with
  own pagination, thread `isResolved` from one GraphQL query (resolved
  threads show as resolved). GitLab: conditional GETs, own client per
  provider. `provider/rate_limit`: a 403/429 (`Retry-After`, reset headers)
  pauses the provider until the reset; `ReviewProvider::rate_limited_until()`;
  `poll/` skips a limited provider with one provider-level error,
  `RefreshError.rateLimitedUntil` (RFC 3339) next to the message; the
  Providers section shows "rate limited until HH:MM". Timeouts on every `gh`
  and GitLab call: a hung one fails that call, the other reviews still update.
- ✅ **Milestone 6 — Keyboard flow.** Frontend only. One focused item on the
  review page (`state/focusStore`; rules and page order in the pure
  `lib/focusModel`): the Conversation, a file, a thread, a finding or a diff
  line, replacing the per-kind "last clicked" (it keeps
  `historyStore.focused`, `aiStore.focused` and `commentStore.focus` in
  step). `usePageFocus` moves the DOM focus and scrolls; the item shows a
  focus ring (`data-focused`). Actions in `lib/keyboardFlow` (nothing there
  posts: a reply or comment only opens a composer). `CommandPalette` (fuzzy
  `lib/fuzzy`, rows `lib/palette`: every action, review actions, Go to
  review…, Go to file…; Approve… and Post all… confirm first) and
  `ShortcutHelp`, one at a time via `overlayStore`. `Modal` takes
  `initialFocus` and focuses on open.
  - Keys (all rebindable): list `↑`/`↓`, `Enter`, `1`–`4`, refresh
    `Shift+R` / `Mod+R` (was bare `r`; a saved override still wins),
    `Mod+K` palette, `?` help; app `Mod+,` settings; files `j`/`k` files,
    `n`/`p` threads and findings, `Enter`/`t` open or fold the thread, `r`
    reply, `h` then/now, `c` comment, `v` viewed, `a` AI panel, `x` select,
    `e` edit, `d` dismiss finding; composer `Mod+Enter` submit, `Esc`
    (fixed) cancel.

## Architecture

A Cargo **workspace** (`members = ["src-tauri", "crates/*"]`).

```
crates/argus-core/   # ALL domain logic; zero Tauri dependency (plain cargo test)
  src/lib.rs         # Core — THE single entry point (all mutation goes through it)
  src/error.rs       # CoreError / CoreResult (app-owned text, never secrets)
  src/ids.rs         # UUID newtypes via uuid_newtype!
  tests/             # integration tests over the public API
src-tauri/           # thin IPC adapter: (de)serialize and forward, nothing else
  src/state.rs       # AppState: Arc<Core> + last RefreshReport
  src/error.rs       # IpcError { kind, message } ← CoreError / PostError
  src/events.rs      # review:changed / reviews:refreshed / ai:state emission
  src/commands/      # review, thread, files, post, history, ai, settings, provider shims
src/                 # React (presentation only)
  ipc/types.ts       # hand-written wire types (mirror the serde derives)
  ipc/commands.ts    # typed invoke() wrappers
  ipc/events.ts      # typed listen() wrappers
  state/             # Zustand stores (mirror the backend; don't guess)
  styles/            # tokens.css + global.css
```

`argus-core` modules: `provider/` (GitLab/GitHub clients, provider status,
conditional-GET cache, rate limits), `git/` (read-only local clone access),
`review/` (rounds), `store/` (SQLite), `settings/`, `poll/`, `files/` (the
code view's file list + viewed marks), `post/` (comments, replies,
approvals; the only writes, explicit only), `history/` (then/now thread
history, line walk, timeline, cache), `ai/` (the AI pass: adapters,
prompt, output validation, runs, findings/checklist/verdicts).

### IPC contract

Commands return `Result<T, IpcError>`, `IpcError = { kind, message }`. `kind`
is a stable camelCase category the frontend branches on; `message` is
sanitized, app-owned text — **never secrets** (tokens!). `CoreError` → `IpcError`
via `From`; IPC-only failures use `IpcError::new` / `IpcError::invalid_input`.

Wire casing is **camelCase** (`#[serde(rename_all = "camelCase")]`; Tauri maps
camelCase JS arg keys to snake_case params). Ids are plain UUID strings. Keep
`src/ipc/types.ts` in lockstep with the Rust side.

### Events

After every refresh (poll or `refresh_reviews`) the shell emits
`review:changed { reviewId }` per changed review, then one
`reviews:refreshed` with the `RefreshReport` (time + per-provider errors).
`last_refresh` returns the latest report for a webview that subscribed late.
After a successful post, reply or approval (or a posted AI finding) the
shell emits `review:changed` for that review too, and after a history call
that moved the review's untouched count.

The AI pass emits `ai:state` with the review's `AiRunState` on every state
change of a run (queued, running, done, failed, cancelled); a run that
ended also emits `review:changed` (its history pass may have moved the
untouched count). The poll callback and `refresh_reviews` hand each report
to `Core::on_refresh_report`, which queues the runs it calls for without
waiting on them.

### Frontend

A store action calls a typed IPC wrapper, then **re-reads from the backend**;
stores also re-read on the events above. The GitLab token only ever goes to
`set_gitlab_token`, never into `Settings`.
Components use design tokens (`src/styles/tokens.css`), never hard-coded
colors/durations/easings.

## Build, run, test

`just` lists recipes. The Tauri CLI is the local dev-dependency (`pnpm tauri …`).

```
just dev            # Vite + Tauri, hot reload
just build          # .app + APFS .dmg (scripts/make-dmg.sh)
just test           # cargo test --workspace + vitest
just test-core      # argus-core only
just lint           # clippy -D warnings + eslint
just format         # rustfmt + prettier
just version-check  # versions in sync across manifests
just coverage       # cargo-llvm-cov + Vitest v8 (not in `just test` or CI)
```

## Testing

No network, no real tokens, fictitious data only (`acme-fixture/*`,
`*-fixture` users, `glpat-fixture-…`), no fixed sleeps (wait on a channel,
`AiRuntime::wait_idle` or `support::tick()`).

- **Core unit tests** sit next to the code (`src/<module>/tests.rs`,
  builders in `src/testutil.rs`); they may reach private state
  (`core.ai.set_executor`, `Store { conn }` for old schemas).
- **Core integration tests** (`crates/argus-core/tests/`) use the public
  API only. Doubles: a fake `gh` shell script per file that routes argument
  lines to `tests/fixtures/github/*.json` and logs calls + token env
  (Unix-only: `#[cfg_attr(windows, ignore)]`); a `TcpListener` GitLab
  double routing paths to `tests/fixtures/gitlab/*.json`; `tests/support`'s
  `Platform`, a scripted provider that logs every call as read or write
  (the guarantee tests); a real temp git repo as the walk's ground truth
  (`history_git.rs`).
- **New provider fixture:** save the anonymised API response under
  `tests/fixtures/<github|gitlab>/`, add its route (the `case` in the fake
  `gh` of `github_provider.rs`, `fixture_routes` in `gitlab_provider.rs`),
  assert on the normalised model, never on raw JSON.
- **Spec guarantees** (`tests/guarantees.rs`, `src/ai/guarantees.rs`): no
  platform write without an explicit post/reply/approve/post_findings
  (refresh, code view, history, polling, settings, AI runs); a review
  leaves the active list exactly when approved; waiting-on-me + reasons
  per status; outdated threads fully readable; re-review to posted comment;
  AI off keeps tracking and history. `tests/token_safety.rs` fuzzes error
  bodies echoing the token through every provider call;
  `tests/history_edges.rs` and `store/tests.rs` (upgrade from every schema
  version) cover the rest.
- **IPC contract:** `src-tauri/tests/ipc_contract.rs` checks every
  `generate_handler!` command against its `commands.ts` wrapper (name +
  camelCased args), that only `src/ipc/` touches `invoke`/`listen`, and the
  event names. `src/ipc/wire-shapes.json` holds the wire structs' key paths
  and enum values: `tests/wire_shapes.rs` (Rust) and `src/ipc/wire.test.ts`
  (literals typed by `types.ts`) both compare against it. After a deliberate
  wire change: `ARGUS_UPDATE_WIRE=1 cargo test -p argus-core --test
  wire_shapes`, then fix `types.ts` until Vitest passes.
- **Frontend** (Vitest + Testing Library, jsdom, `src/test/setup.ts`):
  builders in `src/test/*Fixtures.ts`; mock `@tauri-apps/api/core` /
  `event` per file; `DiffEditor` is stubbed (CodeMirror needs layout).
  `App.flow.test.tsx` runs the whole app over a stateful fake backend.
  Assert that writes happen only on their button (filter `invoke` calls by
  the write commands).

## Conventions

- **Rust:** `#![forbid(unsafe_code)]`; clippy `-D warnings`; `cargo fmt`.
  Comment the _why_.
- **Layering:** domain logic in `argus-core`; if a command handler grows logic,
  move it down. The core must compile without Tauri.
- **Security:** provider tokens never appear in errors, logs (above `DEBUG`),
  or fixtures. Tests use **only fictitious** data.
- **Motion:** `--dur-*` / `--ease-*` tokens only; keep the reduced-motion guard.

## Versioning, CI & releases

- SemVer. `[workspace.package]` version in the root `Cargo.toml` is the source
  of truth; `scripts/sync-version.sh` propagates it (`--check` detects drift).
- `.github/workflows/ci.yml`: macOS job (fmt, clippy, `cargo test --workspace`,
  typecheck, lint, vitest, build), Windows job (`argus-core` fmt/clippy/test),
  a `changelog` job on PRs, and a `release` job on push to main.
- **Every PR adds an entry under `## [Unreleased]` in `CHANGELOG.md`**
  (user-facing voice). Merging to main releases it: patch by default, `minor` /
  `major` PR label bumps more. The first release ships the `Cargo.toml` version.
  `build.yml` (dispatch-only) builds the .dmg and NSIS installers.
- `RELEASE_DEPLOY_KEY` (optional secret) lets the release commit through a
  protected main; without it the job falls back to `GITHUB_TOKEN`.
- Do **not** create tags or releases by hand without explicit approval.
