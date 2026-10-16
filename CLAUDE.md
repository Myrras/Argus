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
    `ProvidersSection`), resizer, the work column (`DetailTabBar` chips
    Threads / Files over the detail island; `WelcomeScreen` without a
    selection), `Modal`, `Toasts` (`toastStore`), `ContextMenu`, lucide
    `icons.tsx`. UI prefs are localStorage-only: `themeStore` → `argus.theme`
    (dark/light/retro), `layoutStore` → `argus.layout`.
  - Files tab (`FilesTab`): placeholder with a collapsible file-tree slot
    (`data-slot="file-tree"`) and a diff slot (`data-slot="diff"`); the diff
    view plugs in there. Next: M3 (then/now
  thread history; the thread card has a `data-slot="then-now"` placeholder).

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
  src/commands/      # review, thread, settings, provider shims
src/                 # React (presentation only)
  ipc/types.ts       # hand-written wire types (mirror the serde derives)
  ipc/commands.ts    # typed invoke() wrappers
  ipc/events.ts      # typed listen() wrappers
  state/             # Zustand stores (mirror the backend; don't guess)
  styles/            # tokens.css + global.css
```

`argus-core` modules: `provider/` (GitLab/GitHub clients, provider status),
`review/` (rounds), `store/` (SQLite), `settings/`, `poll/`. Planned (create
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
