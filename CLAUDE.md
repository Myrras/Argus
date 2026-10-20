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
  - `mine` = `Thread::is_mine`: started by the user or with a non-system
    note of theirs (`startedByMe` tells the first apart).
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
    `settingsRequestStore` (open on a tab from deeper in the tree; an
    Editors tab sits between AI and Shortcuts). Opened by
    the macOS menu's Settings… (`menu:open-settings`, `lib.rs`) or the
    keymap's `openSettings` (Ctrl+, elsewhere). Shortcuts: `lib/keymap`
    (rebindable, bare keys allowed unlike Iris), `useAppShortcuts`,
    `ShortcutsTab`. Local clones use the dialog plugin's folder picker.
  - Review view (one page, no tabs): `ReviewDetail` = header + `ReviewView`
    = `FileTree` (`data-slot="file-tree"`, a Conversation entry first, then
    tree/flat + filter) beside the page (`data-slot="diff"`):
    `ConversationCard` (threads without a code reference, mine first; plus
    "On other files" for anchored threads whose file the list does not show,
    `diffThreads.conversationThreads`; both parts grouped by
    `diffThreads.groupThreads` into Unresolved, Comments (not resolvable)
    and Resolved, folded in card state, opened when `n`/`p` land in it via
    `useRevealCard`; `pageStops` walks the same order; counts:
    `unresolvedCount` as "N open" in the tree entry and card header,
    `resolutionLabel` over all threads in the header's status row) then
    one `FileDiffCard` per file, contents loaded lazily (IntersectionObserver or selection; cached per
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
    base (compare API; a PR too large for it: the local clone's merge base,
    else its base commit flagged `baseApproximate`, whose old side takes no
    comments), GitLab latest version `base_sha`; since-last-review
    = GitHub `compare/last...head` (left: its merge base), GitLab
    `compare?straight=true` (left: last). Viewed marks: `viewed_files`
    (migration 2), keyed path + `content_sha` (GitHub blob sha, GitLab diff
    digest) of the review's own diff. Keys `nextFile`/`prevFile`/
    `toggleViewed` (scope `files`).
  - GitHub conversation (`provider/github_conversation.rs`): issue comments
    and non-empty submitted review bodies are anchor-less threads
    (`issuecomment-<id>`, `pullrequestreview-<id>`); threading by Argus's
    hidden marker (`<!-- argus:reply-to <thread> -->`, last line, honoured
    only on the user's own entries and for an earlier entry of the same
    PR), by quote (`>` block of lines of an earlier entry: whole, or >= 24
    chars of one from a word on; bullets ignored; one line needs >= 12
    chars; alerts and indented `>` are not quotes) or the PR author's very
    next real answer after the user, wherever filed (not `/commands`, not
    remarks to someone else; code is skipped); replies post an
    issue comment quoting the thread's root (`@` defused, HTML skipped) plus
    the marker.
- ✅ **Milestone 4 — Posting.** Core `post/`: `post_comments` (one by one,
  a result per comment in order, moved to the platform's current head),
  `reply_to_thread`, `approve_review` (pinned to the head the user saw:
  GitHub review `commit_id`, GitLab approve `sha`; `headMoved` when the
  stored or platform head differs) and `commentable_lines` (per hunk;
  cached per review + stored head). `PostError { kind, message }` → `IpcError`
  by `kind.as_str()`; new kinds `postRejected`, `lineNotInDiff`,
  `headMoved`. Shell `commands/post.rs` emits `review:changed` after every
  successful write. Only these commands (plus `submit_review` and
  `post_findings`) write, each from a button or shortcut handler; nothing
  auto-posts.
  - Resolving: `resolve_thread(reviewId, threadId, resolved)` →
    `Core::resolve_thread` → `ReviewProvider::set_thread_resolved` (one
    write; then the review is refreshed through the poll path, watermark
    untouched). Only for `Thread::resolvable` (migration 12):
    GitHub review threads (GraphQL `resolveReviewThread` / `unresolveReviewThread` with the node id read
    from `reviewThreads` right before; the PR conversation cannot be
    resolved), GitLab discussions with resolvable notes, not single
    comments (`PUT …/discussions/:id` with `resolved`). Refusals
    (permission) are `postRejected` with the platform's message. UI: a
    Resolve / Unresolve button in `ThreadBlock` (`threadStore.setResolved`:
    busy state, toast on failure, badge after the re-read; the header's
    "N unresolved threads" with ⌃ ⌄ jumps). Resolved threads fold, GitLab
    style: by default everywhere (`historyStore.threadOpen` /
    `thenNowOpen`: open only when mine and not resolved), and when a
    re-read flips a thread's resolved state (`foldOnResolve` sets
    `open` to `!resolved`; the user's own click holds until the next
    flip); a click, Reply, then/now or `h` opens one; `n`/`p` focus the
    header without unfolding. Key `s`
    (`resolveThread`), palette "Resolve thread" / "Unresolve thread"; no
    confirmation (reversible).
  - UI: `DiffEditor` takes `items` (blocks under lines besides threads) and
    `comment` (`CommentGutter`: "+" on commentable lines per side, click /
    shift-click / drag, selection + focused-line decorations); the unified
    view comments on new-side lines only. `FileDiffCard` loads the lines
    (`commentStore.loadCommentable`) and places the composer and drafts.
    `commentStore`: drafts per review (localStorage `argus.drafts.v1.<id>`),
    the one composer, reply composers, the focused line; `CommentComposer`
    (textarea + Preview via `Markdown`) and `CommentBlocks`
    (`InlineComposer`, `DraftBlock`, `ReplyComposer`). Header: pending
    count + Post all, Submit review… with `SubmitReviewModal` (see Review
    state below; it replaced Approve… / `ApproveModal`). Keys `commentLine`
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
  - Duplicates: the prompt includes every existing thread (all reviewers,
    open/resolved/outdated, MR-level, posted findings by id; "Existing
    threads" hard-capped at 60k, "Your open threads" at 40k, GitHub
    conversation entries get no verdict). Each finding and checklist item
    carries `relatedThreadIds` (validated against the prompt's ids).
    `ai/dedupe.rs::classify` scores prose words only (code, fences, inline
    code, identifiers left out; the words of the thread's hunk end and the
    finding's line count only when the texts also share a word of their
    own that is no generic noun such as `input`; stem (doubled consonants
    single), then EN/NL stop words (an agent noun like `gebruiker` is no
    inflection of `gebruik`); ≥ 2 shared words; a title's share in a note
    for titles of ≥ 2 words; a note's share in the finding only for notes
    of ≥ 5 words whose words the title shares too). A repeat: the agent
    marked the thread and the score without the title's share is ≥ 0.3
    (a checklist item also needs ≥ 3 shared words; never for a
    high-severity finding: it stays, with `relatedTo`), or by the net: same
    path/side within ±2 lines (or the line the thread's hunk maps to on the
    other side) ≥ 0.5; MR-level, or a side where the thread has no place at
    all, ≥ 0.7. Kept findings and checklist items carry `relatedTo`: the
    agent's marks, then threads the net finds close (near ≥ 0.3, elsewhere
    ≥ 0.5; UI tag "related to bob's comment" in `FindingsList`,
    `FindingBlock` and `Checklist`). Repeats are stored set aside
    (migration 9: `findings.repeat_reason/_of_thread/_of_finding`,
    `shown_anyway`, `posted_thread_id/_note_id`,
    `ai_runs.skipped_checklist`; migration 10: the same four columns on
    `checklist`, `findings.related_to` JSON; migration 11:
    `checklist.related_to`),
    listed by `list_skipped_findings` / `list_skipped_checklist`, shown by
    `show_finding_anyway` / `show_checklist_item_anyway` (local; kept on a
    rerun of the round for the same path/line/side/title or item text),
    never posted while set aside; the AI panel lists both under "Skipped as
    already raised (N, k high)" (`SkippedFindings`, open with a high one;
    the header chip's tooltip counts the same list). The calibration table
    is `dedupe::tests::calibration`. Posted findings match their thread by
    platform ids, not text.
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
  - Open in editor (Iris's `open_local` + `editorOverrides`): core `open/`
    `local_files` / `local_file` (repository path → the clone's working
    tree: no absolute paths or `.`/`..` parts, symlinks resolved must stay
    inside; `exists`, `cloneHead` via `LocalClone::head` = `rev-parse HEAD`,
    `matchesReviewHead`; `None` without a clone), `open_in_editor` (app =
    `editorOverrides[ext]`, else `defaultEditor`, else the OS default, which
    is refused for files that would run) and `reveal_local_file`, through the
    injected `Opener` (shell `opener.rs`: tauri-plugin-opener `open_path` /
    `reveal_item_in_dir` from Rust; the webview has no open-path
    permission). No line support (Iris has none). UI: `localFileStore`
    (one `local_files` per review + file list, again on window focus),
    `OpenInEditorButton` (tree rows via `FileTree.fileAction`, card
    headers), `lib/useLocalFileMenu` (right-click: Open in Editor / Open
    With Default App / Show in Finder), `lib/localFiles` (tooltips, keys),
    Settings → Editors (`lib/editorOverrides`, Iris's tab + a default
    editor). Keys `openInEditor` (`o`) and `showInFinder` (`Mod+Alt+R`),
    scope `files`, on the focused item's file.
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
  posts: a reply or comment only opens a composer; `s` resolves, the
  thread's own reversible write). `CommandPalette` (fuzzy
  `lib/fuzzy`, rows `lib/palette`: every action, review actions, Go to
  review…, Go to file…; Submit review… and Approve… open
  `SubmitReviewModal`, Post all… confirms first; both rendered once by
  `ReviewDialogs`) and
  `ShortcutHelp`, one at a time via `overlayStore`. `Modal` takes
  `initialFocus` and focuses on open.
  - Keys (all rebindable): list `↑`/`↓`, `Enter`, `1`–`4`, refresh
    `Shift+R` / `Mod+R` (was bare `r`; a saved override still wins),
    `Mod+K` palette, `?` help; app `Mod+,` settings; files `j`/`k` files,
    `n`/`p` threads and findings; next/previous unresolved thread
    (`nextOpenThread`/`prevOpenThread`: resolvable and not resolved, as the
    header counts them; no default key, run by the header's ⌃ ⌄ buttons
    and the palette, a saved binding still applies; no wrap, "No more open
    threads" at an end). Bare `↑`/`↓` are the list's only: on an open
    review's page (`keyboardFlow.onReviewPage`: not the sidebar, unless the
    list's arrows came last) they are left to the browser and scroll (a
    diff's `.cm-content` picks lines), `Enter`/`t`
    open or fold the thread, `r` reply, `s` resolve or reopen, `h`
    then/now, `c` comment, `v` viewed, `a` AI panel, `x` select, `e` edit,
    `d` dismiss finding, `Alt+R` refresh this review (`refreshReview`);
    composer `Mod+Enter` submit, `Esc` (fixed) cancel.
  - Refresh one review: `reviewStore.refreshOne` (busy per review in
    `refreshingOne`, never blocks `refreshing`; applies the returned record,
    re-reads the list when changed; failures toast, a rate limit with its
    time) from the header's refresh `iconButton` beside the platform link,
    the row menu's Refresh (the row shows a small turning glyph, `aria-busy`),
    the palette's "Refresh this review" and `Alt+R`.

- ✅ **Review state — the platform decides the tabs; Submit review.**
  Providers report the user's own reviewer state (`MyReviewState`:
  `awaitingReview`, `reviewStarted`, `requestedChanges`, `commented`,
  `approved`, `notReviewer`; GitLab `/reviewers` `state`, GitHub
  `requested_reviewers` + latest standing review), when they last submitted
  a review on the platform and at which head (`my_reviewed_at/sha`: GitHub
  the latest review's `submitted_at`/`commit_id`; GitLab the newest
  "approved this merge request" / "requested changes" system note of theirs
  from `notes?sort=desc`, one page, at the diff version current then) and
  the newest "requested review from @me" note (`review_requested_at`,
  parsed by `gitlab::requested_reviewers`). Migration 13 stores the three
  `my_*` columns (12 is the thread resolve's). `review::classify`: approved
  → Approved; closed → Closed; owed (awaiting / started) → Needs review,
  or Needs re-review with `reRequested` once the user reviewed on the
  platform or a request is newer than Argus' watermark (never an edge
  between polls); otherwise the later of Argus' watermark and the platform
  review is the watermark (a dated verdict without either waits on the
  author). `rerequested_at` = max(edge, platform). Core
  `post/submit.rs`: `Core::submit_review(id, verdict, seen_head, summary)`
  (head checks as approve; GitHub `POST pulls/{n}/reviews` `event` +
  `commit_id` + `body`, summary required for request changes / comment;
  GitLab approve = `approve`, request changes = the `/request_changes`
  quick action note after `GET /version` ≥ 17.2 and a reviewer check,
  verified by the 202 and `/reviewers`, else `unsupported` — REST
  `bulk_publish reviewer_state` is 19.2+ only — then the summary as a
  note; a user who is not a reviewer is added first, the one extra write:
  GraphQL `mergeRequestSetReviewers` `operationMode: APPEND` (bearer
  token; server-side merge, no REST `reviewer_ids` race; not
  `/assign_reviewer me`, which GitLab drops silently without the
  permission), then `/reviewers` read until it lists them
  (`REVIEWER_RECHECK`, `with_reviewer_recheck` in tests); a refusal, Free's
  one-reviewer limit or no listing is `postRejected` "Argus couldn't add
  you as a reviewer…" with GitLab's reason. Test double:
  `tests/support/gitlab.rs` (`submitting`, `Add`), shared by
  `post_gitlab.rs` and `guarantees.rs`); an approval marks every file of the diff at that head viewed
  (`Store::set_viewed_all`). `approve_review` delegates to it. The local
  `mark_reviewed` stays core-only (no IPC). UI: `SubmitReviewModal`
  (verdict radios per `lib/reviews.verdictsFor`, summary Write/Preview,
  "Post N pending comments with it" via `commentStore.postAll`, aborting
  and listing refused drafts), `reviewStore.submit` (success clears
  `selectedId`: the work column shows `WelcomeScreen`) + `justApproved`
  (`APPROVED_LINGER_MS` "Approved ✓" in its old tab, kept across list
  reloads), overlay confirm `{ kind: "submit", verdict? }` (the dialog's
  one owner: the header button, ⇧S, the palette and the list's menu only
  `ask`; `ReviewDialogs` in `App` renders it, so a second request replaces
  the open dialog, never stacks one), key `submitReview` (⇧S, scope
  `files`).

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
  src/commands/      # review, thread, files, post, history, ai, settings, provider, open shims
  src/opener.rs      # the core's Opener: tauri-plugin-opener, called from Rust
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
code view's file list + viewed marks), `open/` (a clone's file in the
user's editor), `post/` (comments, replies,
approvals, resolving threads; the only writes, explicit only), `history/` (then/now thread
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
`refresh_review(reviewId)` (`Core::refresh_review`, `poll::refresh_review`)
re-reads one review through `persist` (no list call, its `listed` kept):
it takes the refresh lock like any refresh (waits for a poll under way; a
poll waits for it), joins one that read the review after the ask
(`Store::fetched_since`, ticket taken before the wait; then `changed:
false`), and never calls a rate-limited provider (checked before and after
the wait). Returns `ReviewRefresh { review, changed }`; the shell emits
`review:changed` when changed, hands `report()` to `on_refresh_report`, and
leaves `last_refresh` alone (the global report). A rate-limited
`CoreError` maps to IpcError kind `rateLimited` with the bare "… rate
limited until HH:MM" text. The post path's own refetch
(`refetch_after_write`) takes no lock; tickets order it.
After a successful post, reply, approval or resolve (or a posted AI finding) the
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
just dev-signed     # debug .app, stable signing identity (no hot reload)
just build-signed   # release .app, stable signing identity
just install-signed # build-signed + copy to /Applications
just test           # cargo test --workspace + vitest
just test-core      # argus-core only
just lint           # clippy -D warnings + eslint
just format         # rustfmt + prettier
just version-check  # versions in sync across manifests
just coverage       # cargo-llvm-cov + Vitest v8 (not in `just test` or CI)
```

### macOS keychain prompt during development

`just dev` ad-hoc signs the binary, and that signature changes on every
rebuild, so macOS re-asks for keychain access to the GitLab token (service
`com.argus.app.gitlab`) each run; "Always Allow" can't stick. Dev-only: a
properly signed release reads its items silently.

- `just dev-signed`, `just build-signed` and `just install-signed` sign with a
  stable identity from the gitignored `src-tauri/tauri.dev-signed.conf.json`
  (only `bundle.macOS.signingIdentity`), so one "Always Allow" persists.
  `dev-signed` has no hot reload; keep `just dev` for fast iteration.
- The conf is machine-specific: copy it from Selene/Podium or create it.
  List identities with `security find-identity -v -p codesigning`.

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
  platform write without an explicit post/reply/approve/resolve/post_findings
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
