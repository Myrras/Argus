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
    rendered by `SuggestedChange`; never applied. The gutter "+" is the disabled M4 comment hook. Core: `fetch_files`
    (`DiffRange::Review` / `Compare`) per provider; left side = GitHub merge
    base (compare API), GitLab latest version `base_sha`; since-last-review
    = GitHub `compare/last...head` (left: its merge base), GitLab
    `compare?straight=true` (left: last). Viewed marks: `viewed_files`
    (migration 2), keyed path + `content_sha` (GitHub blob sha, GitLab diff
    digest) of the review's own diff. Keys `nextFile`/`prevFile`/
    `toggleViewed` (scope `files`).
  - Next: M3 (then/now thread history; the thread card's "Then/now
    history" toggle expands the `data-slot="then-now"` placeholder).

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
  src/error.rs       # IpcError { kind, message } ← CoreError
  src/events.rs      # review:changed / reviews:refreshed emission
  src/commands/      # review, thread, files, settings, provider shims
src/                 # React (presentation only)
  ipc/types.ts       # hand-written wire types (mirror the serde derives)
  ipc/commands.ts    # typed invoke() wrappers
  ipc/events.ts      # typed listen() wrappers
  state/             # Zustand stores (mirror the backend; don't guess)
  styles/            # tokens.css + global.css
```

`argus-core` modules: `provider/` (GitLab/GitHub clients, provider status),
`review/` (rounds), `store/` (SQLite), `settings/`, `poll/`, `files/` (the
code view's file list + viewed marks). Planned (create
them when their milestone starts): `history/`, `ai/`.

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
```

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
