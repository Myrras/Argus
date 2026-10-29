# CLAUDE.md — Argus

<!-- rust-base:start -->
<!-- Synced from rust-base. Do not edit this block here; change it in rust-base. -->
## Shared CI and recipes (rust-base)

CI, release and installer builds come from
<https://github.com/RobbieMinderhoud/rust-base>. `.github/workflows/` holds thin
callers (`ci.yml`, `pr-checks.yml`, `build.yml`, `integration.yml`) of its
reusable workflows, pinned `@v2`. The callers and the shared config files
(`.gitattributes`, `.editorconfig`, `rust-toolchain.toml`, `.prettierrc.json`,
`.prettierignore`, `eslint.config.js`, `tsconfig*.json`,
`.github/dependabot.yml`) are synced from rust-base: **change them there**,
not here. Behaviour is documented in the rust-base README.

App-specific CI settings (runner, Windows test scope, extra recipe, release
title prefix, integration packages and env) live in `.github/rust-base.json`.
That file is owned by this repo.

Dependabot runs monthly with minor/patch grouped (Cargo and npm); PRs carry
`release:patch`. Majors of app-specific dependencies come as separate PRs. Shared-stack majors
(React, Tauri, Vite, ESLint, ...) are ignored here: rust-base decides them.

- **Required checks on `main`:** `ci / ci-ok`, `pr-checks / Changelog entry`,
  `pr-checks / Release label`. Don't require `ci / frontend`, `ci / rust` or
  `ci / windows`. The labels `release:patch`, `release:minor`, `release:major`,
  `no-release` and `windows` must exist.
- **Release labels:** every PR carries exactly one of `release:patch`,
  `release:minor` or `release:major`, enforced by `pr-checks.yml`. Use
  `no-release` for CI, config or docs-only PRs. Merging to `main` releases:
  the version is the latest `v*` tag bumped by the highest label across the PRs
  merged since. Never bump the version or create a tag by hand; `just version`
  is for emergencies only.
- **Changelog gate:** every PR adds an entry under `## [Unreleased]` in
  `CHANGELOG.md` (user-facing changes only, _Keep a Changelog_). `pr-checks.yml`
  fails unless the PR touches `CHANGELOG.md` and that section holds at least one
  `- ` entry. The heading must stay exactly `## [Unreleased]`. `no-release` PRs
  need no entry.
- **`packageManager`:** don't add one to `package.json`. The shared workflows
  set pnpm 10; a second version fails setup.
- **Shared recipes** (`dev`, `dev-signed`, `build`, `build-signed`,
  `install-signed`, `check`, `test-core`, `format`, `version`, `version-check`,
  `rust`, `frontend`, `ci`, `sweep`) come from `ci/common.just` in rust-base.
  The justfile imports it with `import? '~/.rust-base/ci/common.just'`; link
  your clone with `ln -s /path/to/rust-base ~/.rust-base`. Don't redefine those
  names (just rejects duplicates) and never add `set allow-duplicate-recipes`:
  change the shared recipe in rust-base instead. Recipes that CI calls
  (`test-integration`) must not depend on shared recipes: in CI the optional
  import resolves to nothing.

### When to add the `windows` label

Windows CI runs on pushes to `main` and on PRs that carry the `windows` label
(adding the label re-runs CI for the PR). It gates the release. Add `windows`
to a PR when it:

- touches `cfg(windows)`, `cfg(unix)` or `target_os` code, or platform-specific modules
- changes file paths, path separators, drive letters or case sensitivity; process spawning, PTY or shell quoting; environment variables; WSL
- changes the Windows settings in `tauri.conf.json` (bundle, NSIS) or installer scripts
- adds or upgrades a dependency with platform-specific code (`tauri`, `windows-rs`, `portable-pty`, …) or bumps the Rust toolchain
- fixes a Windows bug

Skip it for frontend-only, docs-only and purely cross-platform logic changes. If unsure, add it: `main` runs Windows after the merge anyway, and a failure there blocks the release.
<!-- rust-base:end -->

## Windows-sensitive spots

Add `windows` to a PR that touches these (all in `crates/argus-core/src/` unless noted):

- `platform.rs`: finding `gh`, `glab`, `claude`, `auggie` (login shell on Unix, `where` and `%COMSPEC%` on Windows), `CREATE_NO_WINDOW`
- `git/mod.rs` (git lookup, process spawning), `provider/github.rs` (spawning `gh`)
- `ai/adapter.rs`: agent spawning, sandbox and absolute-path checks (`cfg!(windows)`)
- `open/mod.rs` and `src-tauri/src/opener.rs`: opening files in an editor, what runs instead of opens (`RUNS_ON_OPEN_WINDOWS`)
- `provider/gitlab.rs` (spawning `glab`), `accounts/cli.rs` (`gh`/`glab auth status`): `CREATE_NO_WINDOW`
- `scripts/run.rs` and `scripts/save.rs`: running submit scripts (own process group on Unix, `CREATE_NO_WINDOW`) and saving them executable (Unix mode bits)
- `git/discover.rs`: looking for local clones under the home folder (`USERPROFILE` on Windows)
- `src-tauri/src/lib.rs` (macOS-only menu bar; the title-bar window controls and Settings gear elsewhere), `src-tauri/src/main.rs` (`windows_subsystem`)
- `src-tauri/tauri.conf.json` (bundle, icons) and `build.yml` (NSIS installer)
- tests with Unix fake scripts (`fake gh`, `fake glab`, fake agents, submit scripts, git fixtures; written with `tests/support/script.rs` `write_executable`) carry `#[cfg_attr(windows, ignore)]`; the Windows job runs clippy on the whole workspace but tests `-p argus-core` only, so `src-tauri` tests run on Linux only

Guidance for AI agents (and humans) working in this repository.

## What Argus is

Argus is a desktop **review cockpit for GitLab merge requests and GitHub pull
requests**, built in **Rust + Tauri 2 + React + TypeScript + Vite**.
macOS-first; Windows kept in mind.

## Boilerplate (shared skeleton)

The skeleton — Cargo workspace shape, `justfile`, `scripts/`, CI, shared config
files, design tokens, layering/IPC conventions — comes from
<https://github.com/RobbieMinderhoud/rust-application-boilerplate>
(`../rust-application-boilerplate` locally). CI, release and the shared
recipes and scripts come from
<https://github.com/RobbieMinderhoud/rust-base> (see Versioning, CI &
releases).

- **Rule:** a change to the **shared skeleton or conventions** (not Argus
  domain code) also goes as a **PR against the boilerplate**.

## Status

- ✅ **Milestone 1 — Skeleton.** Renamed boilerplate, CI + release pipeline.
- ✅ **Milestone 2 — Providers and the review list.** GitHub (`gh`) and GitLab
  (`glab`) providers, SQLite store, settings, round detection,
  background polling; IPC commands + events; the four-tab list, detail view
  with threads (mine first, outdated never hidden), settings, provider
  status. The boilerplate greet/notes sample is gone.
  - Both providers go through the user's CLI and its login; Argus holds no
    platform token (no keychain, nothing in `settings.json` or the IPC).
    GitLab (`provider/gitlab.rs`): every REST call is `glab api --hostname
    <host> --include --method <M> <path>` (body as JSON on stdin via
    `--input -`; GraphQL is the `graphql` path), no shell, cwd the temp dir,
    env = the user's plus `NO_COLOR`, `GLAB_NO_PROMPT`,
    `GLAB_CHECK_UPDATE=false`, `GLAB_SEND_TELEMETRY=false`, minus
    `GLAB_DEBUG_HTTP`/`DEBUG` (they print the `PRIVATE-TOKEN` header); never
    a token variable. glab prints status line + headers + body for any
    status (exit 1 for a non-2xx, a 304 and a GraphQL `errors` answer too),
    parsed by the shared `provider/http.rs` (`HttpReply`, also `gh api -i`'s);
    no status line = no response, classified from stderr (never forwarded):
    `dial tcp` = not connected (retried, writes too: nothing was sent),
    "has not been authenticated"/401 = `GitlabAuth { host }` ("Not logged
    in to <host>: run `glab auth login --hostname <host>`"), spawn failure =
    `GlabMissing` (IPC kinds `gitlabAuth`, `glabMissing`). glab follows
    redirects itself (with its token, even across hosts: glab's behaviour).
    `GitlabProvider::new(base_url)` (built from an account: `<scheme>://
    <host>`, glab's `api_protocol` else https; the host, port included, is
    `--hostname`; scheme and path only make web links),
    `with_binary` (tests), binary found like `gh` (`platform::glab_binary`:
    login shell `command -v`, then well-known paths). GitLab's error
    messages pass `redact_tokens` (GitLab token prefixes `glpat-`, `gloas-`,
    …, 64-hex OAuth tokens, whatever follows `Bearer`/`PRIVATE-TOKEN:`/
    `access_token=`…) before `sanitize`: Argus never knows glab's token.
  - Accounts (`accounts/`): Argus has no host settings; every host the
    user's CLIs are logged in to is an `Account { key: "provider:host",
    provider, host, user?, state: loggedIn | notLoggedIn{message?} |
    error{message} | cliMissing{hint}, active? (gh's active account),
    accounts (gh: every login on the host, for the picker), account? (the
    chosen one), enabled (an entry decides; without one: on only when loggedIn, so a
    host never used, like glab's gitlab.com, is off, not polled, no warning;
    a missing CLI is on only when an enabled entry of its provider exists), scheme? (glab's api_protocol), loginCommand }`.
    `accounts/cli.rs` asks `gh auth status --json hosts` (`{"hosts":
    {"<host>": [{state: success|error|timeout, active, login, …}]}}`; an
    older gh without `--json`: its text, `Logged in to <host> account
    <login>` / `as <login>`, `Active account: true`) and `glab auth status
    --all --output json` (`{"hosts": [{host, state: success|error, user,
    error, api_protocol, …}]}`; without `--all` / `--output` it retries
    and reads the text), both in parallel, 30 s each, never
    `--show-token`, only the named fields read; gh's error texts are never
    forwarded (Argus' own words), glab's pass `redact_tokens` +
    `sanitize`. glab error with 401/unauthorized/"no token" = notLoggedIn,
    dial/timeout/x509 = error ("could not reach"). A missing CLI is one
    `cliMissing` row for its provider (install hint); garbage is one
    error row, and the CLI's last good answer is kept (asked again next
    read). `accounts::list(settings, discovered)`: every reported host,
    plus the hosts only `Settings::accounts` names (notLoggedIn); GitHub
    first, then by host. A chosen gh account the host does not have is
    notLoggedIn. `provider_specs`: one `ProviderSpec { provider, host,
    account?, baseUrl }` per enabled host whose state is loggedIn or error
    (could not be checked: keep polling). `AccountBook` (an `Arc` in
    `Core`) caches the answer: asked the first time (the shell's startup
    thread calls `list_accounts(false)`, then starts polling; a provider
    use before that asks first), on `list_accounts(true)` (Check again),
    after a settings change of `accounts` (`mark_stale`) and after a
    refresh whose provider-level error for a host is `unavailable` (401 /
    glab auth failure / CLI missing; `Core::note_refresh`, poll callback
    and `refresh`, `refresh_review`); a host with no provider-level error
    in a refresh goes error → loggedIn without asking (`note_reachable`).
    A core over explicit providers (`with_providers*`, tests) has
    `Source::Fixed` (its providers as logged-in accounts, never a CLI);
    `Core::with_cli(store, settings, gh, glab)` runs given programs
    (tests: fakes). `Core::list_accounts(recheck)` also rebuilds the
    providers when the specs changed. IPC `list_accounts(recheck)`
    (`commands/provider.rs`).
  - Providers: one per enabled host (several GitLab or GitHub hosts at
    once), `Core.providers` = `Built { spec, provider }`; a rebuild keeps
    the instance of an unchanged spec (rate-limit pause, conditional
    cache, user), builds new specs through `build_providers`
    (`provider::build(spec, (gh, glab))`; a fixed core picks from its
    pool). `update_settings` rebuilds only on an `accounts` change (from
    the cached answer) and always restarts a running loop.
    `RefreshError.host` names the provider's host. `Core::reviews()` (the
    list) leaves out reviews of a host switched off
    (`Settings::account_enabled`); they stay stored and come back when it
    is switched on; `provider_for` has no provider for them ("… is not set
    up: check Settings → Accounts").
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
  - Settings as in Iris: `SettingsModal` (tabs General / Accounts / AI /
    Repositories / Editors / Scripts / Shortcuts; switches, steppers and
    choices apply at once, text fields in a form with its own Save, failures
    as toasts), `settingsStore` (UI prefs →
    `argus.settings`: reduced motion, shortcut overrides),
    `backendSettingsStore` (the core's `settings.json`, `accounts` from
    `list_accounts`: `refreshAccounts` on load, after every save and every
    `reviews:refreshed`, `checkAccounts` = Check again, `setAccount(key,
    choice)` moves the row at once and saves `Settings.accounts`;
    `setupChanged` per provider whose account entries a patch changed; no
    token passes: gh and glab keep theirs), Settings → Accounts
    (`AccountsTab`: the explanation, one card per account with platform
    badge, host, state (`lib/accounts.stateLabel`), on/off switch "Use
    GitLab at <host>" applied at once, a gh account `select` when the host
    has several, for notLoggedIn / cliMissing the login command in a
    `CodeBlock` with copy; "Add an account": a host field filling `gh auth
    login --hostname <host>` and `glab auth login --hostname <host>`,
    Check again), `settingsRequestStore` (open on a tab from deeper in the
    tree, `open(tab, target?)`: `repo` for Repositories, `account` for
    Accounts, which scrolls to and highlights that card; `seq` counts
    requests, so the same one asked again still switches the tab).
    `ProvidersSection` lists the enabled accounts (`lib/providers`) and
    `lib/banner` items (not logged in with its command, could not check,
    CLI missing, no accounts yet, refresh errors as "GitLab (<host>): …");
    its Settings button opens Accounts on the item's account. Opened by
    the macOS menu's Settings… (`menu:open-settings`, `lib.rs`) or the
    keymap's `openSettings` (Ctrl+, elsewhere). Shortcuts: `lib/keymap`
    (rebindable, bare keys allowed unlike Iris), `useAppShortcuts`,
    `ShortcutsTab`. The AI tab holds the pass itself (on/off, the read-only
    warning), the default review language (`ReviewLanguageField` without
    "Default") and a link to Repositories.
  - Settings → Repositories (`RepositoriesTab`): one card per repository
    from the core's `list_repositories` (`repos/`: `RepoSetup { key, host,
    repo, provider?, openReviews, clonePath?, clone: CloneState (notSet |
    found | missing | notRepository: the folder and its `.git`, a `git` call
    only without one), reviewSkill, reviewLanguage? }`; every stored
    review's repository, closed ones too, plus every key in `clonePaths`,
    `reviewSkills` or `reviewLanguages`; open reviews first, then by key
    case-insensitive; a settings-only repository's provider from the
    enabled accounts, `accounts::enabled_hosts`; stored reviews of a host
    switched off still count). `repositoriesStore` re-reads on open, after every
    settings change and on `reviews:refreshed` while the tab shows (its own
    subscription), so a new repository appears with a quiet "Not set up"
    (`lib/repositories.isSetUp`: no clone, skill Auto, no language). Per
    card: platform badge, "N open", Local clone (short path, state,
    Choose… via the dialog plugin's folder picker, Show =
    `reveal_repo_clone(repo)`, the folder resolved by the core, Clear), Review skill
    (`ReviewSkillField` in `ReviewSkillPicker`: "Change" opens a filterable
    radio list, Auto and None on top, then the skills by source; reads
    `list_review_skills` through `reviewSkillsStore`; pure rules in
    `lib/reviewSkills`), Review language (`ReviewLanguageField`: a select
    Default (<global>) / Auto / English / Nederlands / Other…, applied at
    once; Other… has a name field with its own Save). A filter from 7
    repositories (`FILTER_FROM`); "Add repository" (key + folder, checked
    by `lib/clonePaths.cloneError`) for one without reviews. Opened on a
    repository (`settingsRequestStore.open("repositories", key)`: the clone
    chip, the clone suggestion's "Set up…"), the tab clears the filter,
    scrolls the card into view and highlights it (`data-highlighted`).
  - Review view (one page, no tabs): `ReviewDetail` = header + `ReviewView`
    = `FileTree` (`data-slot="file-tree"`, a Conversation entry first, then
    tree/flat + filter) beside the page (`data-slot="diff"`):
    `ConversationCard` (threads without a code reference, mine first; plus
    "On other files" for anchored threads whose file the list does not show,
    `diffThreads.conversationThreads`; both parts grouped by
    `diffThreads.groupThreads` into Unresolved, Comments (not resolvable)
    and Resolved, folded in card state, opened when `n`/`p` land in it via
    `useRevealCard`; `pageStops` walks the same order; counts:
    `unresolvedCount` as "N open" in the card header (the tree counts only
    your open threads, `myOpenCount`: Conversation "N open", a pill per file),
    `resolutionLabel` over all threads in the header's status row) then
    one `FileDiffCard` per file, its diff mounted only near the viewport
    (see Big reviews), contents cached per review/sha/path in
    `codeViewStore`, binary and large (>1500 lines or no patch) diffs as
    cards ("Load anyway"). `codeViewStore.selected`/`order` hold the
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
    `suggestion` = start_line..line, GitLab `suggestion:-N+M`, either part
    optional as in GitLab's `SuggestionsParser`; original lines from
    `file_contents` at the anchor's commit, else `diff_hunk`) rendered by
    `SuggestedChange` (`anchor` + `source` props); never applied. Writing
    them: `CommentComposer`'s `suggestion` prop (`lib/useComposerSuggestion`:
    the plan, the head's lines via `loadContents`, the file's indent, a
    preview `renderFence`) gives the toolbar's "Insert suggestion"
    (`iconButton`, `aria-disabled` + the reason as tooltip) and key
    `insertSuggestion` (`Mod+G`, scope `composer`): `insertSuggestion`
    puts ` ```suggestion ` (GitHub) or ` ```suggestion:-N+0 ` (GitLab, N =
    lines − 1 back from the note's `new_line`, the range's last; at most
    100) at the caret on its own lines, the head's lines inside and
    selected, a longer fence when they hold ``` (GitLab's
    `repeatCodeBackticks`). Where: `commentSuggestionPlan` (new-side
    lines; not old-side, GitHub `outside: fallback`, binary / too large /
    patchless files) and `replySuggestionPlan` (the thread's lines: GitHub
    `currentLine` back by its `startLine` span at the review head, GitLab
    the discussion's own lines; not review-level, outdated, old-side,
    `outsideDiff` or file-level threads); a load failure is a reason too.
    Inside a block's content (`suggestionAt`) Enter keeps the indent and
    Tab / Shift+Tab use `indentUnit` (`enterInSuggestion`,
    `tabInSuggestion`, through `insertText` so undo works). Preview and
    `DraftBlock` render blocks as `SuggestedChange` at the head
    (`planAnchor`). `suggestionMismatch` warns when a block does not cover
    the comment's lines (GitLab offsets; GitHub: `composer.suggestedFor`,
    the range at insert, against a range changed since), fixed by
    `retargetSuggestions` ("Update suggestion range", GitLab) or
    `setComposerRange` (GitHub: back to the suggestion's lines). The body
    posts unchanged (`post_github.rs` / `post_gitlab.rs` suggestion
    tests). AI findings carry no replacement code, so "Post" sends none. Core: `fetch_files`
    (`DiffRange::Review` / `Compare`) per provider; left side = GitHub merge
    base (compare API; a PR too large for it: the local clone's merge base,
    else its base commit flagged `baseApproximate`, whose old side takes no
    comments), GitLab latest version `base_sha`; since-last-review
    = GitHub `compare/last...head` (left: its merge base), GitLab
    `compare?straight=true` (left: last). Viewed marks: `viewed_files`
    (migration 2), keyed path + `content_sha` (GitHub blob sha, GitLab diff
    digest) of the review's own diff. Keys `nextFile`/`prevFile`/
    `toggleViewed` (scope `files`).
  - Hide whitespace changes (frontend only; neither platform's API takes a
    whitespace option for the patch: GitHub `pulls/{n}/files` and the
    compare API take `page`/`per_page`, GitLab `merge_requests/:iid/diffs`
    and `changes` `page`/`per_page`/`unidiff`/`access_raw_diffs`; `?w=1` is
    their web UIs'). `layoutStore.hideWhitespace` (`argus.layout`, default
    off, every review), the toolbar's toggle after the layout switch (reads
    "Whitespace hidden" while on), key `toggleWhitespace` (`w`, scope
    `files`, palette row from the keymap). `git diff -w` semantics: lines
    compared with all whitespace removed (`lib/whitespace.whitespaceKey`);
    blank lines added or removed still count. `lib/whitespaceDiff`
    (CodeMirror, loaded with `DiffEditor`): `HIDE_WHITESPACE` =
    `DiffConfig.override` passed as `diffConfig` to `MergeView` /
    `unifiedMergeView`; line runs by key through the library's `diff` over
    one character per distinct key, then inside changed runs the plain
    character diff minus spaces/tabs at a change's ends and spans that are
    whitespace on both sides (same line breaks); `settle` reruns the result
    through `presentableDiff` (`Chunk.build` applies `makePresentable`,
    which slides pure insertions over text it assumes equal) and slides a
    chunk that moved by a line of the same key, else widens it to whole
    lines. Documents untouched. `DiffEditor` prop `hideWhitespace` rebuilds
    the editors like a layout switch (reading line kept);
    `collapseSync.mirrorPositions` and `diffThreads.mapOldLine` map by
    lines (unchanged lines may differ in whitespace). Files:
    `lib/whitespace` (pure, no CodeMirror) reads the patch per hunk
    (`patchWhitespaceStats`, LCS on keys) for the tree's and the card's
    +/− (`shownLineStats`) and `whitespaceOnly`; contents, once loaded,
    decide in the card (`onlyWhitespaceDiffers`). A whitespace-only file is
    an "Only whitespace changes" card with "Show changes" (per card, its
    editor without the override) unless threads, drafts, the composer or
    findings sit on its lines (then the editor, everything folded but
    around them, under a note); the tree shows a quiet ␣ instead of +/−.
    Viewed marks keep `content_sha`; commenting and posting are unchanged.
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
  stored or platform head differs) and `commentable_lines` (per hunk,
  plus `outside`; cached per review + stored head). `PostError { kind, message }` → `IpcError`
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
  - Comments outside the diff: `ReviewProvider::outside_diff()` →
    `OutsideDiff` (`none` default, `native` GitLab, `fallback` GitHub),
    reported per file as `CommentableLines.outside` (`none` without a patch
    or for an added/removed file). `post::resolve_in` places a line outside
    every hunk as a `Context` row with both numbers (`anchor::outside_row`:
    shifted by what the hunks above added or removed), `PlacedComment.in_diff`
    `false`; a range stays in one `Region` (one hunk, or one stretch between
    hunks: `region_of`, the gap index counts hunks whose side lines all lie
    above; the code view counts the same), else `invalidInput` with words
    ("…can't be one comment: it mixes changed lines with unchanged code
    outside the diff…"); `lineNotInDiff` stays for real impossibilities.
    GitLab posts it as any position (both `old_line`/`new_line`; GitLab's
    `Position#diff_file` unfolds the diff around an unchanged position, for
    modified/renamed files, lines within the blob). GitHub refuses a review
    comment outside the diff, so `post_comment` sends a file comment
    (`subject_type: "file"`, no line) written by
    `provider/github_outside.rs`: "[Lines 12–18](https://<host>/<repo>/blob/
    <head>/<path>#L12-L18) of `path` (outside the diff):", a fenced excerpt
    with line numbers (`PlacedComment.excerpt`: the core reads the head's
    lines through the clone-aware `Core::file_at`, at most 40, shown 15; an
    unreadable file posts without it, a line past the end fails
    `lineNotInDiff`), the comment, then `<!-- argus:lines <head> 12-18 -->`
    on the last line. Read back (`github_outside::apply` in
    `fetch_threads`), only on the user's own root file comment: the anchor
    gets the lines on the new side, `outside_diff: true` (wire
    `anchor.outsideDiff`), the note shows the user's text only; outdated
    once the PR head (`pulls/{n}`, conditional) is not the marker's. A
    file-level comment (`subject_type: "file"`) is no longer outdated for
    lacking a diff position. An old-side line outside the diff goes as its
    new-side line.
  - UI: `DiffEditor` takes `items` (blocks under lines besides threads) and
    `comment` (`CommentGutter`: commentable lines per side, optional
    `regionOf` / `outsideHint`, click / shift-click / drag, selection +
    focused-line decorations). One "+" follows the pointer (`hoverField`;
    per editor, so per side in the split view; `elementAtHeight` picks the
    text line, nothing over block widgets, collapsed stretches or removed
    chunks; gone on mouseleave; drawn as a gutter marker beside a fixed
    spacer, so no layout shift, no fade). Its tooltip "Click to comment,
    drag to select lines" (+ the outside hint), `ns-resize` cursor. A drag
    (`dragField`, `lib/commentDrag.dragReach`): the range follows the
    pointer within the anchor's region and stops before folded lines
    (`cm-argusDragLine`, start/end edged; `dragStop` says why it stopped),
    a line it can't reach is marked (`cm-argusDragBlocked`), the
    `<html data-comment-drag="ok|blocked">` cursor and a floating
    `.argus-comment-drag-label` ("Comment on lines 12–18", or "… · a range
    in the diff can't leave its hunk", role status) in `global.css`;
    release comments on the reached range, Escape cancels. The unified view
    comments on new-side lines (unchanged ones too); removed lines in the
    split view. Split view: `syncCollapse` mirrors every
    `uncollapseUnchanged` to the sibling (`lib/collapseSync.mapCollapsePos`,
    the library's `mapPos`), so a stretch Argus opens for a thread, the
    composer, the focused line or the reading line opens on both sides; a
    mousedown on a collapsed stretch no longer focuses a line in it.
    `FileDiffCard` passes every line (`commentableSet(…, docLines)`) when
    the platform takes outside lines; the composer and drafts carry
    `outside` (`InlineComposer` says what it becomes, `DraftBlock` "(outside
    the diff)"); `ThreadBlock` / `ListedThread` show "on lines 12–18
    (outside the diff)" (`diffThreads.outsideDiffLabel`). `FileDiffCard` loads the lines
    (`commentStore.loadCommentable`) and places the composer and drafts.
    `commentStore`: drafts per review (localStorage `argus.drafts.v1.<id>`),
    the one composer, reply composers, the focused line; `CommentComposer`
    (textarea + Preview via `Markdown`) and `CommentBlocks`
    (`InlineComposer`, `DraftBlock`, `ReplyComposer`). Header: pending
    count + Post all, Submit review… with `SubmitReviewModal` (see Review
    state below; it replaced Approve… / `ApproveModal`). Keys `commentLine`
    (`files`), `submitComment` / `cancelComment` (new `composer` scope;
    Escape is `fixed`). Ranges: `lib/commentRanges` (one region: a hunk,
    or one stretch outside the diff; `regionOf`, `clampRange`).
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
    `Timeline` (notes, commits, resolves; long comments fold; the notes are
    the stored thread's, merged by `threadTimeline` with the history's
    commits and resolves, so a reply shows with the threads' re-read),
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
  - Auggie (`ai/auggie.rs`): which auggie a binary is comes from its
    `--help` (stdout, else stderr: 1.2 prints it there into a pipe), read
    once per canonical path + size + mtime (`probe`, `HELP_TIMEOUT` 30 s;
    only a fitting answer is cached), never from the version. **Current**
    (1.x, checked 1.2.0-prerelease; needs `--print`, `--mode` with `json`,
    `--tools`, `--no-tools`, `--mcp-config`, `--remove-tool`,
    `--workspace-root`): `--print --mode json --no-session`, prompt on
    stdin, `--workspace-root` the temp dir, `--mcp-config
    {"mcpServers":{}}` (the user's MCP servers off; theirs write),
    `--no-extensions --no-skills --no-rules --no-prompt-templates` when
    listed, review `--tools read,grep,find,ls`, writer `--no-tools` +
    `--append-system-prompt SYSTEM`, and `--remove-tool=` for every name
    Argus knows (`CURRENT_REMOVED_TOOLS` + the legacy list; extension tools
    such as `web-search`, `agent`, sub-agents and `ask-user` are not
    covered by `--tools`) minus `CURRENT_READ_TOOLS`. No `--add-workspace`:
    with Augment's multi-repo context (on, 2026-10) an added workspace's
    `AGENTS.md`/`CLAUDE.md` and its parents' load as instructions, and the
    worktree's are the head's; the prompt names its path and the read tools
    take absolute paths. Answer: `--mode json` lines
    (`adapter::parse_auggie_line`, `OutputKind::AuggieJson`): the last
    assistant message of `message_end`/`agent_end` (`stopReason` error /
    aborted / cancelled = failed), `message_update` `thinking_delta` /
    `text_delta` and `auto_retry_start` to the sink. **Legacy** (0.x:
    `--instruction-file`, `--remove-tool`, `--workspace-root`): as before,
    `--print` + `--quiet --ask --dont-save-session` when listed, prompt
    file, text answer, `tools list` first (fail closed). Never `tools list`
    to a current one: it takes the words as a prompt and runs with every
    tool. A help that fits neither, or fails, is `AgentError::Unsupported`
    ("this auggie version can't run read-only: its --help lists no
    `--mcp-config`"); nothing runs. Unknown flags fail the run (both
    shapes refuse them). `read_only_guaranteed` stays false: no path rules
    for the read tools, and the user's own auggie hooks run.
    `AUGGIE_ENABLE_TERMINAL` / `AUGGIE_ENABLE_TOOL_SEARCH` are kept out of
    the env.
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
  - Review context (`ai/context.rs`, `ai/skills.rs`, `job.rs`): with a
    clone, a run reads a **review worktree** at the head
    (`git::worktree`, see Local clone) through `--add-dir` / a legacy
    auggie's `--add-workspace` (a current auggie: by path in the prompt)
    instead of the clone (sandbox unchanged: temp cwd,
    deny rules around it, `disableAllHooks`, `--setting-sources user`,
    auggie's read tools); without one, or when the fetch or worktree step
    fails, the API diff with the clone read-only as before, and the run's
    note says "Couldn't create a review worktree: …; reviewed without
    it". **Rules and skills come from the target branch, never the head**
    (the author controls it): `BaseFiles` reads at `record.base_sha`
    (else a full diff's exact left side) from the clone's objects
    (`files_at`, `list_dir`), else the API's `file_at`. Rules: `CLAUDE.md`,
    `AGENTS.md`, `.github/copilot-instructions.md`, `.cursorrules`,
    `.cursor/rules/*.mdc` (clone only), then the `CLAUDE.md` nearest each
    changed file (API: at most 8 probes), 40k in all, the last one cut
    with a marker, the rest listed. A path the review changes that is a
    rules or `.claude/skills/` file (`rules_changed`; whole review: the
    clone's merge-base diff, else the run's diff) puts a note in the prompt
    and the run state (`listed`: 10 paths, then a count); the base's
    version is used. Claude also gets `Skill` in `--disallowedTools`, so a
    skill the checkout ships is never loaded. Skills
    (`SkillSource`): the user's (`<claude dir>/skills/*/SKILL.md`,
    `~/.claude` or `CLAUDE_CONFIG_DIR`), installed plugins'
    (`plugins/installed_plugins.json` v1/v2 → `<installPath>/skills/*`,
    named `plugin:skill`), the project's (`.claude/skills/*` at the base;
    without a clone only `code-review` and `review` are tried); a review
    skill's name (frontmatter `name`, else the directory) contains
    "review". `Settings::review_skills` (`host/repo` →
    `ReviewSkillChoice` `auto` | `none` | `skill { source, name }`, read
    leniently, `auto` not stored) is resolved at run time; Auto = the
    project's exact `code-review`, else a `code-review` directory, else the
    first; a chosen skill that is gone falls back to Auto with a note. The
    skill goes into the prompt as data under `SKILL_GUIDANCE` (apply its
    criteria; ignore posting, pushing, scripts, writes and its answer
    format; the answer stays Argus' JSON), never through CLI skill loading,
    so claude and auggie get the same; `SKILL.md` up to 30k plus at most 3
    `.md`/`.txt` files of its own directory that it links to or quotes
    (8k each, `..`, absolute paths and links out refused). What a run was
    given is `RunContext` (`rulesFiles`, `skill`, `worktree`, `language`,
    `notes`), in
    `AiRunState.context` (migration 14: `ai_runs.context` JSON, written by
    `save_done`, kept while the next run runs) and the AI chip's tooltip
    (`lib/findings.aiContextLines`). `Core::list_review_skills(repo)` /
    IPC `list_review_skills` (read-only; the project's at the clone's
    `origin/HEAD`, else `HEAD`); `Core::set_claude_dir` (an in-memory core
    has none; `Core::open` sets the real one).
  - Review language: `ReviewLanguage` (`auto` | `english` | `dutch` |
    `other { name }`), `AiSettings::review_language` (the default, Auto)
    and `Settings::review_languages` (`host/repo` → language; no entry =
    the default; read leniently: an unreadable global value is Auto, an
    unreadable entry is left out; `validated` makes a name one line,
    trimmed, at most 40 chars, a blank `other` the default).
    `Settings::review_language(review)` resolves it at run time; the prompt
    gets `prompt::language_instruction` once, after the task, before any
    data: every human-readable field (finding titles and bodies, checklist
    items, verdict reasons, the prose around suggested code) in that
    language, never code, identifiers, paths or the JSON keys and fixed
    values; Auto = `AUTO_LANGUAGE` ("the language the merge request's title
    and description are written in; if that is unclear, English"). The run
    records it in `RunContext.language` (`None` for older runs); the AI
    chip's tooltip says "Language: …". `dedupe` is unchanged: a finding in
    another language than the thread shares no prose words, so the net
    cannot tell and an agent-marked one is kept with `relatedTo`
    (`a_dutch_finding_on_an_english_comment_is_kept_and_names_it_when_marked`).
- ✅ **Milestone 6 — Local clone.** Core `git/`: with a clone configured per
  repository (`Settings::clone_paths`), the history walk and `Core::file_at`
  read through plain `git` (`rev-list`, `diff-tree`, `cat-file --batch`)
  instead of the API: faster, exact, never truncated. Reads are read-only
  by construction: object reads only (no checkout, fetch or status),
  `GIT_OPTIONAL_LOCKS=0`, no lazy fetch of a partial clone (git >= 2.44),
  no shell, 30 s timeout, an env without tokens or `GIT_*` redirections.
  Always a fallback, never a requirement: an unfetched commit, shallow
  boundary, type change, non-UTF-8 file or any git failure is answered by the
  API, per thread or per file (`git::file_or`; `compute_histories` takes
  `clone: Option<&Path>`). Argus never changes the clone, with one
  exception: for an AI run it fetches the review's head and manages its
  own worktrees under `.argus/worktrees`, nothing else.
  - Review worktrees (`git/worktree.rs`, `LocalClone::review_worktree`):
    under a per-clone lock, sweep stale ones, fetch the head when the
    clone lacks it (`git fetch --quiet --no-tags --no-write-fetch-head
    --no-recurse-submodules --refmap= <remote> refs/pull/<n>/head` or
    `refs/merge-requests/<iid>/head`, then the sha itself; the remote whose
    `remote -v` fetch URL is the review's `host/repo`, else `origin`),
    add `/.argus/` to the common dir's `info/exclude` once, write the
    sidecar marker `<slug>.argus` (`argus-review-worktree v1`), then
    `git worktree add --detach .argus/worktrees/<provider>-<n>-<sha7>-<run8>
    <head>`. The `ReviewWorktree` guard removes it on drop (`git worktree
    remove --force`, the leftover directory, the sidecar, then `git
    worktree prune` only when every prunable worktree is Argus' own, so a
    worktree of the user's on an unmounted volume is never pruned); a run
    drops it however it ends. Sweeps (startup via
    `Core::sweep_review_worktrees` on a thread, and before each run) remove
    marked ones only, never one a run of this process uses (the `live`
    set); nothing else is touched. Changing commands run with
    `core.hooksPath=/dev/null` (a husky `post-checkout` of the branch never
    runs), `submodule.recurse=false`, `core.fsmonitor=false`, `gc.auto=0`,
    `maintenance.auto=false`, `GIT_LFS_SKIP_SMUDGE=1`, and `write_env`:
    `git_env` (no tokens, no `GIT_*` redirections, no prompt) plus
    `GIT_SSH`/`GIT_SSH_COMMAND`/`GIT_SSH_VARIANT`/`GIT_ASKPASS` and the
    login shell's `PATH`, so `SSH_AUTH_SOCK`, `HOME` and credential helpers
    work. Timeouts: fetch and add 120 s, remove 60 s; cancellable
    (`run_with` polls the run's flag). The user's clone needs fetch access
    for heads it lacks.
  - Open in editor (Iris's `open_local` + `editorOverrides`): core `open/`
    `local_files` / `local_file` (repository path → the clone's working
    tree: no absolute paths or `.`/`..` parts, symlinks resolved must stay
    inside; `exists`, `cloneHead` via `LocalClone::head` = `rev-parse HEAD`,
    `matchesReviewHead`; `None` without a clone), `open_in_editor` (app =
    `editorOverrides[ext]`, else `defaultEditor`, else the OS default, which
    is refused for files that would run; with an editor set and
    `Settings::open_with_repository` on (default, Settings → Editors switch
    "Open the repository with the file") `Opener::open_project_file(app,
    root, file)` hands the clone root over with the file: macOS one
    `/usr/bin/open -a <app> <root> <file>`, the trait default and Windows
    the file only; the OS default app always gets the file only) and
    `reveal_local_file`, through the
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
  - Clone indicator: core `open/` `clone_status(reviewId)` →
    `CloneStatus { configured, path, found, head, matchesReviewHead }`
    (read-only: `rev-parse HEAD` through `LocalClone`; `found` = the folder
    exists and is a git repository, `.git` also counts when git is missing)
    and `reveal_clone` (the set folder only, through the `Opener`; no new
    webview permission). UI: `CloneChip` ends the header's meta row, fed by
    `cloneStatusStore` (re-read on open, on a head or `clonePaths` change
    and on window focus; a failed read leaves the chip out). Found: the
    short path (`lib/cloneStatus.shortPath`, `~/…/elements`), tooltip with
    the full path and HEAD against the review's, click = Show in Finder.
    Set but missing or no repository: warning tint, click opens Settings →
    Repositories on the repository (`settingsRequestStore.open(
    "repositories", key)`). Not set: a quiet "No local clone", same
    click. `CloneSuggestionBar`'s "Set up…" opens the same. Not on list rows: it would cost a git call per row.
- ✅ **Milestone 6 — Provider polish.** Polling is polite. `provider/conditional`:
  in-memory ETag / Last-Modified cache per provider (LRU, bounded), so
  unchanged requests are a 304 (free on GitHub). GitHub: `gh api -i` with
  own pagination, thread `isResolved` from one GraphQL query (resolved
  threads show as resolved). GitLab: `glab api --include` with own
  pagination (`X-Next-Page`), conditional GETs (`--header If-None-Match`),
  30 s per glab call, connection failures retried 0/500/1500 ms
  (`with_connect_retries` in tests); a file whose diff GitLab collapsed (empty `diff`, flagged
  `collapsed` only since 18.4) is read once more through
  `changes?access_raw_diffs=true` (`fill_collapsed`), so it has a patch and
  can take comments; a failed raw read leaves it patchless as before. `provider/rate_limit`: a 403/429 (`Retry-After`, reset headers)
  pauses the provider until the reset; `ReviewProvider::rate_limited_until()`;
  `poll/` skips a limited provider with one provider-level error,
  `RefreshError.rateLimitedUntil` (RFC 3339) next to the message; the
  Providers section shows "rate limited until HH:MM". Timeouts on every `gh`
  and `glab` call: a hung one is killed and fails that call, the other
  reviews still update.
- ✅ **Milestone 6 — Keyboard flow.** Frontend only. One focused item on the
  review page (`state/focusStore`; rules and page order in the pure
  `lib/focusModel`): the Conversation, a file, a thread, a finding or a diff
  line, replacing the per-kind "last clicked" (it keeps
  `historyStore.focused`, `aiStore.focused` and `commentStore.focus` in
  step). `usePageFocus` moves the DOM focus and, for a thread or finding,
  reveals it (`lib/revealItem`: joins the file's load,
  `codeViewStore.prefetchFile`, which also pins the card's diff mounted,
  `mountStore`; asks the file's `DiffEditor` via
  `lib/diffEditors` to scroll the block's line into view, as CodeMirror
  renders only lines near the viewport; waits for the block, then puts its
  top under the sticky headers; the card only if none can show); the item
  shows a focus ring (`data-focused`). Only navigation scrolls: the column
  anchors itself (`lib/scrollAnchor`, `overflow-anchor: none`; WKWebView
  has no CSS anchoring; anchors noted once after each scrolled frame, not
  in the scroll event, which would lay the page out twice) and navigation
  pauses it (`pauseScrollAnchor`);
  rebuilt block widgets estimate their last measured height
  (`diffEditors.blockHeight`); a layout switch puts the reading line back,
  a viewed fold keeps its header. Actions in `lib/keyboardFlow` (nothing there
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
    and the palette, a saved binding still applies; they wrap around in page
    order with a toast "Wrapped to the first/last open thread", none for a
    single open thread; "No more open threads" only when there is none). Bare `↑`/`↓` are the list's only: on an open
    review's page (`keyboardFlow.onReviewPage`: not the sidebar, unless the
    list's arrows came last) they are left to the browser and scroll (a
    diff's `.cm-content` picks lines), `Enter`/`t`
    open or fold the thread, `r` reply, `s` resolve or reopen, `h`
    then/now, `c` comment, `v` viewed, `a` AI panel, `x` select, `e` edit,
    `d` dismiss finding, `w` hide whitespace changes, `Alt+R` refresh
    this review (`refreshReview`);
    composer `Mod+Enter` submit, `Esc` (fixed) cancel, `Mod+G` insert
    suggestion;
    app `Mod+F` find
    (`find`, also from fields and the code; from the sidebar the list's
    `[data-list-filter]` field when it has one, it has none yet); scope
    `find` (new: only while the find bar is open, from anywhere; a key the
    focused element handled first wins, so a composer key may share one,
    `keymap.scopesShareKeys` for the conflict check and the defaults test)
    `Mod+G` / `Mod+Shift+G` next / previous match.
  - Find (Cmd+F): `FindBar` pinned under the page toolbar in the column's
    flow (`data-find-bar`; the column's `data-find-open` sets
    `--page-sticky-top`, which the cards' sticky headers and
    `scrollAnchor.stickyBarsHeight` follow): field (the selection
    prefilled), "3 of 27" (`findInReview.findStatus`, "Searching 12 of 40
    files…", an invalid regex inline), ↑/↓, Match case / Whole word /
    Regex, scope select (Everything / Code only / Comments only), "N files
    not searched" with reasons, Close; Enter / Shift+Enter next / previous,
    wrapping with a "Wrapped" hint; Escape closes and gives the focus back.
    `state/findStore`: the bar's state and the run (debounced 150 ms; a
    newer run or closing drops an older one): comment text parsed first in
    ~8 ms slices, every card searched with what is in, then the files'
    contents loaded through `codeViewStore.loadFileSides` (six at a time,
    from the reading position on, priority `soon`), each diffed with
    `Chunk.build` and the editors' own config
    (`whitespaceDiff.editorDiffConfig`, or the removed lines would differ
    from what the editor shows) into `PreparedCode`
    (cached per review/sha/path), results flushed at most every 100 ms,
    capped at 10000. Typing jumps to the first match at or after the
    reading position (`findReveal.readingPlace`) once the files up to it
    are searched; the current match is kept by identity across re-reads
    (threads, files, findings re-run it). Going to a match:
    `focusStore.focusFound` (focus as a click would, the card selected
    with `codeViewStore.select(…, { scroll: false })` so `ReviewView` does
    not scroll, `reveal` opens a folded card or Resolved group, a thread is
    opened), a code line also becomes `commentStore.focus` (the `c` key),
    then `findReveal.revealMatch` (like `revealItem`, no DOM focus: the
    editor's `revealLine` (opens the collapsed stretch, mirrored to the
    sibling) or `reveal` of the block, then `placeInView` when the
    highlight is not in view). Pure model `lib/findInReview` (`FindMatch {
    kind: code|thread|note|finding, card, file?, side?, line?, threadId?,
    noteId?, findingId?, unit, ordinal, start, end, row }`, page order as
    `pageStops`: Conversation, then per file outdated threads, findings
    without a line, unplaced threads, then by row the code and the items
    under each line; code = every head line plus the removed base lines of
    the chunks, a removed run sitting above its chunk's first head line),
    `lib/findQuery` (one global regex; whole word by `\w` lookarounds;
    matches per line, empty ones skipped), `lib/markdownText` (a comment's
    text = the hast text nodes react-markdown renders, raw HTML and markers
    left out; `rehypeFindMarks` wraps matches in `<mark>` inside links too,
    sets `data-find-base` on a fence's `pre`). Highlighting: `Markdown`
    `findUnit` (+ `data-find-unit`), `FindText` (thread author, finding
    title), `CodeEditorBlock` `find`; CodeMirror through `lib/findBridge`
    (no CodeMirror import in the store) and `lib/findHighlight`
    (`findState` StateField per editor, created from and broadcast by the
    bridge, `findTarget` facet `diff` / `text`, decorations over the
    visible lines; a base side only on its chunks' lines; the unified
    view's deletion blocks via the CSS Custom Highlight API with
    `StaticRange`s, near the window only, max 3000 lines; `findKeys` at
    `Prec.highest` in every editor turns Mod+F to the bar). Tokens
    `--find-match`, `--find-current`, `--find-current-ring`,
    `--findbar-h`.
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
  GraphQL `mergeRequestSetReviewers` `operationMode: APPEND` (through
  `glab api graphql`; server-side merge, no REST `reviewer_ids` race; not
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
- ✅ **Big reviews.** Hundreds of files and tens of thousands of changed
  lines open in about a second and scroll smoothly.
  - Disk cache (core `cache/`, migration 15 `content_cache(key, kind,
    body, size, used_at)`): a file's content at a commit (key `file` +
    provider + host + repo + sha + path) and a review's file list with its
    patches (key `diff` + provider + host + repo + number + `review|compare`
    + its two shas) never change, so `Core::file_at` (clone first, then the
    cache, then the provider, then stored) and `cached_diff` (`list_files`,
    `commentable_lines`, `keys_for`) read them once. Only full 40/64-hex
    shas are cached (a branch name moves), only a diff of the head its key
    names (`cache::cacheable`), only content or the normalised `FileDiff`
    (never headers, errors, tokens; `tests/big_review.rs` greps the db).
    A clone's reads are not cached. LRU by `used_at` (a counter): over
    `Settings::cache_limit_mb` (default 512, clamped 32..16384) a write
    evicts down to 90 %; a lower limit trims at once. `refresh_review`
    forgets the review's file lists. `commentable_lines` no longer fetches
    the diff a second time. Batch commands `file_contents_batch` (≤ 64,
    `Core::files_at`: one `cat-file --batch` per commit for a clone, the
    API's on `FILE_READS` = 6 threads, each entry its own result),
    `commentable_lines_batch`, `warm_file_cache` (reads into the cache,
    sends nothing back), `cache_stats`, `clear_cache`; per-entry results
    are `Batched<T>` = `{ ok } | { error: IpcError }`. Settings → General
    → Cache (`cacheStore`): the limit (stepper over `CACHE_LIMITS_MB`), what
    it holds, Clear cache. `big_review_timings` (ignored) measures it.
  - Reads (`lib/requestQueue`): contents and commentable lines go through a
    queue per kind: requests of one tick join one batch call per review
    (24 contents, 32 lines), priority `visible` > `soon` (find) > `idle`,
    the newest visible first (after a jump, what is on screen now), at most
    3 calls at once, idle only when nothing else waits; a key asked twice
    shares one read. `codeViewStore` keeps contents in memory up to
    `MEMORY_CHARS` (LRU) over the core's cache. Tests answer the batch
    commands through their single ones: `test/batches.answerBatches`.
  - Idle prefetch (`lib/prefetch`, `ReviewView`): once per diff, without a
    clone, the files not in memory are warmed into the core's cache in idle
    time, 8 files per `warm_file_cache`, one call at a time, only while no
    contents read waits; stops on unmount or a failed batch; `warmed`
    remembers them for the session.
  - Virtual page (`FileDiffCard`): the card (header, fold, "Load anyway",
    "Show changes") is always there; its body (`FileDiffBody`: contents,
    editor, threads, comments, findings) mounts within 1.5 screens of the
    viewport (IntersectionObserver on the column, margin `150%`), stays
    until 3 (`300%`), and also while selected, focused (`focusStore`),
    holding the composer, or pinned (`mountStore.pinCard`: reveals —
    `prefetchFile`, `useRevealCard`, an AI finding's reveal — for
    `PIN_MS`, dropped once the card is near). Elsewhere a placeholder
    (`data-slot="unmounted-diff"`) of the body's last drawn height
    (`lib/cardHeight`, per review/path/layout/whitespace) or an estimate
    from the patch (hunk rows, a fold per gap, closed threads), and the
    section gets `content-visibility: auto` (skipped layout and paint).
    Opened stretches survive a remount (`diffEditors.rememberOpened` /
    `openedStretches`, per path/side/layout/whitespace/contents);
    composer, drafts, replies and open threads live in their stores.
    `ReviewView` preloads the `DiffEditor` chunk with the file list.
  - Large files (`lib/lineDiff`): from 4,000 characters the merge views
    diff lines first (patience: lines unique on both sides anchor, Myers
    between them), then `presentableDiff` inside each changed run;
    `@codemirror/merge`'s character diff alone turned such files into one
    chunk (nothing folded, every removed line drawn). `PLAIN_DIFF` /
    `editorDiffConfig` in `whitespaceDiff`, used by the editors and find.
  - File tree: rows are memoized (`FileRow` gets only its own file,
    selection and count; the page's callbacks are stable) and
    `content-visibility: auto`, so j/k re-render two rows of a thousand.

- ✅ **Submit scripts.** Core `scripts/`: `Settings::submit_scripts`
  (`SubmitScript { id, name, command, args, verdicts, repos, defaultOn,
  timeoutSecs }`; read leniently: an unreadable entry or verdict is left
  out, missing or repeated ids get a UUID; `tidy` on save: trims, repos
  each once without a trailing slash, timeout clamped 5..600, default 60,
  a blank command kept). `Core::submit_review_with_scripts(id, verdict,
  seen_head, summary, script_ids)` = `submit_review`, then only on success
  `plan` (ticked ids in settings order; one not for the verdict, not for
  the repo — `host/repo`, case-insensitive, empty = all — or gone is a
  `skipped` result) queued on `ScriptRuntime` (one worker thread, batches
  in order, scripts serial; the submit never waits); `on_scripts_done`,
  `wait_scripts_idle`; `shutdown` kills the running one, drops the rest,
  reports nothing. Runner injected (`runtime::Runner`, tests fake it);
  the real one `run::execute`: command = absolute, `~/…` or a bare name on
  the login `PATH` (relative paths refused), spawned directly (no shell)
  with its args in the clone (else a fresh `argus-script-*` temp dir), env
  = Argus' own minus `is_token_var` and foreign `ARGUS_*`, the login
  shell's `PATH`, plus `ARGUS_PROVIDER/HOST/REPO/NUMBER/URL/TITLE/VERDICT`
  (wire values)`/HEAD_SHA/CLONE`; stdin = the same as JSON plus `summary`,
  then closed; own process group, a timeout kills the group (`/bin/kill`,
  no `unsafe`); stdout/stderr tails of 16 KB, never logged. `ScriptStatus`
  `ok | failed | timedOut | skipped | notFound | notExecutable` with an
  app-owned `message` ("… is not executable — run chmod +x …").
  "Write with AI" = script drafts (`scripts/drafts.rs`, `DraftRuntime`):
  `ScriptDraft { id, scriptId?, title, base?, turns: DraftTurn { prompt,
  at, result?: GeneratedScript { name, verdicts, script, explanation },
  error? }, status idle | running | failed | cancelled, waiting, error?,
  createdAt, updatedAt, revision }`, memory only (gone on quit).
  `start_script_draft(description, scriptId?)` (Edit with AI: `base` =
  the saved script via `read_submit_script`'s rules, linked to it),
  `send_script_draft(draftId, message)` (a follow-up; `conflict` while it
  runs), `cancel_script_draft`, `list_script_drafts`,
  `discard_script_draft` (stops it, reports nothing), `link_script_draft`
  (after the first save), `retry_script_draft` (Retry: the failed or
  stopped last request again, its material gathered afresh; `conflict`
  while running or when it has its script); each returns at once, the run on its own thread;
  at most `MAX_RUNNING` = 2 at once, the rest `running` + `waiting`;
  nothing stops a draft but the user (or `shutdown`: kills, drops, reports
  nothing). `on_script_draft` (called under the drafts lock, so per-draft
  order holds) → `script-draft:state`; `revision` (global counter) lets
  the webview keep the newer copy. Title = latest result's name, else the
  base's, else the first request cut to 60. Messages ≤ 8000 chars. A
  turn's thread first attaches what the request names (`attach.rs`,
  read only): a merge/pull request link (`parse_link`: GitLab
  `…/-/merge_requests/<iid>`, GitHub `…/pull/<n>`) of a host with a
  provider (an enabled account, `AttachContext.providers` = the core's
  current ones) → `fetch_review` + `fetch_threads` (title, author,
  state, description, the 5 latest non-system comments on the whole
  review, 1.5k each; 20k cap; at most 3 links), else a note ("Couldn't
  fetch MR !9: …", "Not fetched: … (host is switched off in Settings →
  Accounts | host is not logged in; see Settings → Accounts | host is not
  set up in Argus)", from `AttachContext.hosts` = `accounts::list` over
  the CLIs' cached answer, never asked here); an
  absolute or `~/…` path inside a `clonePaths` folder or the scripts
  folder → the file (no `.`/`..`, lexically and after `canonicalize`
  inside a root, a text file, 30k cap; at most 5), else "Not read: <path>
  (outside your local clones | a link that leads outside … | not found |
  not a text file …)". `DraftTurn.attachments` (labels) and `notes` go on
  the wire, the material (`#[serde(skip)]`) stays in memory for this and
  later prompts (`MATERIAL_BUDGET` 80k, newest request's first, the rest
  named). One turn (`generate.rs` `generate`): the AI pass' adapter and
  binary, no tools at all (Claude `--tools ""` + every tool denied,
  `--strict-mcp-config`, `--setting-sources user`, deny-outside settings,
  hooks off, and `--system-prompt` = `SYSTEM`; auggie per `ai/auggie.rs`:
  current `--no-tools`, no MCP, every known tool removed,
  `--append-system-prompt SYSTEM`; legacy every listed + known tool
  removed; fail closed either way, and `script_generation_status` gives
  the unsupported reason), empty temp cwd, token-free env, prompt on
  stdin then closed (legacy auggie: a file), `GENERATE_TIMEOUT` 300 s, killed on cancel. **Why
  the system prompt:** with Claude Code's own one and no tools, Claude
  (2.1.309) wrote tool calls as text (`<invoke name="Bash">glab mr view
  …`) for a request with an MR link or a path, then kept going or the
  stream stalled: 4 of 6 runs passed 300 s; with `SYSTEM` 4 of 4 answered
  in 59–76 s. `ToolCallGuard` watches the streamed text for tool-call
  markup (`<invoke`, `<function_calls`, `<parameter name=`, `<tool_use`),
  kills the agent at the first one and the repair retry says so; twice
  is "… tried to run commands instead of answering …". Claude streams
  (`--include-partial-messages --thinking-display summarized`; the latter
  is not in `--help` and without it the thinking text is empty; a CLI
  that refuses either, `unknown option` on stderr, is asked again without
  them); `adapter::run_observed` hands `parse_stream_event`s
  (`thinking_delta`, `text_delta`, `thinking_tokens`, `api_retry`) to a
  sink and keeps a 4 KB stderr tail (never logged); `run_tracked` is it
  without either (`run_capped`: a larger stderr cap, for auggie's help).
  A current auggie streams through the same sink (`--mode json`); a
  legacy one streams nothing Argus reads: stages only.
  Errors name the agent: "claude took longer than 300s …", "claude exited
  with status N: <stderr tail>" (`stderr_tail`: last 3 lines, control
  characters out, tokens redacted, 300 chars), "claude answered, but not
  with a script (…)". Progress: `ScriptDraft.progress`
  (`ScriptDraftProgress { turn, stage: DraftStage reading | starting |
  thinking | writing | validating | repairing, startedAt, thinking (last
  `MAX_THINKING` 20k, redacted on the way out), thinkingCut,
  thinkingTokens, answerChars, notice }`, set while the turn runs, gone
  when it ends: the thinking is not kept) and `script-draft:progress`
  (`ScriptDraftProgressEvent`, `Core::on_script_draft_progress`, at most
  every `PROGRESS_EVERY` 100 ms per draft via `Throttle`, a stage change at
  once, under the drafts lock); the prompt
  states the Scripts tab's contract (POSIX sh, `set -eu`, the variables,
  stdin, cwd, own `glab`/`gh`, no tokens or fixed hosts, `jq` checked,
  idempotent, stderr + exit code, comments) plus the configured platforms,
  then the conversation: the base, every earlier request in order with
  its script (or "failed" / "stopped"), script versions within
  `HISTORY_BUDGET` (40k, the latest always in full, the oldest dropped
  first, a note says how many; requests never dropped), the new request;
  the answer is the whole updated script, validated strictly (four keys,
  `#!` start), one repair retry, token shapes redacted (and said). The
  prompt opens with "You have no tools…", its contract adds the checklist
  rule (read the description, tick only the template's items by their
  text, leave other `- [ ]` lines, write back only on a change), and the
  attached material comes before the requests under `MATERIAL` (data, not
  instructions; an example: never hard-code its number, title or texts),
  then what could not be attached.
  `script_generation_status` (AI off / agent not found → reason). New
  `CoreError` kinds `agent`, `cancelled`, `conflict`. Saving (`save.rs`):
  `save_submit_script(name, content, overwrite)` → `<app data>/scripts/
  <slug>.sh`, mode 0755, atomic, `conflict` unless `overwrite`, names with
  a folder refused; `read_submit_script(id)` only inside that folder;
  `submit_scripts_dir` / `reveal_submit_scripts_dir` (the `Opener`).
  Shell: `submit_review` takes `scriptIds`; `commands/scripts.rs`;
  `scripts:done` event. Guarantees: nothing runs on refresh, polling, AI
  runs, settings changes, a refused/invalid submit or a submit without the
  tick (`scripts/tests.rs`, `tests/guarantees.rs`); the GitLab token (glab's)
  never reaches a script (`tests/guarantees.rs`, real process); a draft
  writes, saves and runs nothing.
  - UI: Settings → Scripts (`ScriptsTab`, between Editors and Shortcuts;
    `settingsRequestStore` `"scripts"`): the explanation (when they run,
    the `ARGUS_*` table, stdin, cwd, timeout, no shell / `#!` / `chmod +x`,
    own logins and no token, "only add scripts you trust", a collapsible
    example via `Markdown`'s `CodeBlock` with copy), the scripts folder with
    Show in Finder, one card per script (name / command with the dialog
    plugin's file picker / args one per line in a form with its own Save;
    verdicts, repo chips with suggestions from `clonePaths` and reviews,
    Ticked by default, timeout `Stepper`, move, Remove apply at once), the
    last run per card. `SettingToggle` / `Stepper` live in
    `SettingsControls`. "Write with AI" / "Edit with AI" (only for a script
    inside the scripts folder; a script with a linked draft opens that
    draft) open `ScriptWriter`, a conversation: the turns in order (your
    request, then the explanation and the script via `CodeBlock`; older
    versions folded "Version N: …", the latest open; the base as "The saved
    script you start from"; "Writing…" / "Waiting…" / "Stopped." / the
    error), under each request "Attached: …" and the notes; the running
    turn shows the stage (`lib/submitScripts.stageLabel`), the elapsed time
    (`formatElapsed`, ticking each second), the answer size or thinking
    tokens, Stop, the API-retry notice and a "Thinking" block (`Thinking`:
    `<details>` open while it streams, follows the end while you are at
    the bottom); when the turn ends `submitScriptsStore.thoughts` keeps its
    thinking in this window only, shown folded as "Thought for 1m 05s"; a
    failed or stopped last request has Retry (`retry_script_draft`). A
    composer at the bottom ("Describe what the script should do",
    then "Ask for changes…"; Mod+Enter = `submitComment` binding;
    "Keeps running in the background — you can close this."),
    name + verdicts from the latest result, "Read the script before you save
    it…", Save script → `save_submit_script` (a `conflict` asks "Replace
    it?"; a linked script's own file is replaced), the settings entry, then
    `link_script_draft`; Discard; Close never stops the run. Disabled with
    the status' reason. Drafts section above the cards (title, spinner +
    `draftStatusLabel`, request count; Stop / Open / Discard); a linked
    card shows "Draft with changes — Open"; a running row adds its latest
    thought or notice on one line (`progressSnippet`); a spinner on the Scripts tab
    while one runs. `submitScriptsStore` (dir, generation status, `drafts`
    from `list_script_drafts` + `script-draft:state` via `upsertDraft`,
    `script-draft:progress` via `withProgress` (only the running turn's),
    discarded ids ignored, the writer `{ draftId, scriptId, session }`;
    a draft that stops running while its writer is not open toasts
    "Script ready: …" with Open (`showDraft`: `settingsRequestStore`
    `"scripts"` + the writer) or the failure),
    `scriptResultsStore` (latest result per script from `scripts:done`;
    toasts: an error per failure with the last stderr lines, one success,
    one quiet skipped note), pure rules in `lib/submitScripts` (matching
    mirrors the core). `SubmitReviewModal`: "Run after submitting" for the
    scripts of the repo and verdict, ticks from `defaultOn` kept per id
    across verdict changes, "Manage scripts" (closes the dialog, opens the
    tab); no scripts at all → one quiet line; the ticked ids go through
    `reviewStore.submit` → `submitReview(…, scriptIds)`.

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
  src/events.rs      # review:changed / reviews:refreshed / ai:state / scripts:done / script-draft:state / :progress
  src/commands/      # review, thread, files, post, history, ai, settings, provider, open, repos, scripts shims
  src/opener.rs      # the core's Opener: tauri-plugin-opener, called from Rust
src/                 # React (presentation only)
  ipc/types.ts       # hand-written wire types (mirror the serde derives)
  ipc/commands.ts    # typed invoke() wrappers
  ipc/events.ts      # typed listen() wrappers
  state/             # Zustand stores (mirror the backend; don't guess)
  styles/            # tokens.css + global.css
```

`argus-core` modules: `accounts/` (what gh and glab are logged in to, the
account list, provider specs, the cache), `cache/` (the disk cache of
file contents and file lists), `provider/` (GitLab/GitHub
clients, conditional-GET cache, rate limits), `git/` (local clone access: object
reads, plus the AI pass' review worktrees),
`review/` (rounds), `store/` (SQLite), `settings/`, `repos/` (the
repositories Argus knows and their setup), `poll/`, `files/` (the
code view's file list + viewed marks), `open/` (a clone's file in the
user's editor), `post/` (comments, replies,
approvals, resolving threads; the only writes, explicit only), `history/` (then/now thread
history, line walk, timeline, cache), `ai/` (the AI pass: adapters,
prompt, output validation, runs, findings/checklist/verdicts), `scripts/`
(submit scripts: matching, the queue and process runner, "Write with AI",
saving to Argus' `scripts/` folder).

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

The submit scripts of one successful `submit_review` report once they all
ended: `scripts:done { reviewId, results }` (`ScriptsDone`, one
`ScriptResult` per ticked id), through `Core::on_scripts_done`. Nothing
else emits it.

A "Write with AI" draft reports every change (running, waiting, started,
idle, failed, cancelled, linked) as `script-draft:state` with the whole
`ScriptDraft`, through `Core::on_script_draft`; a discarded draft and
anything after `shutdown` report nothing. While a turn runs, its progress goes out as
`script-draft:progress { draftId, progress }` (paced, see Submit scripts),
through `Core::on_script_draft_progress`.

### Frontend

A store action calls a typed IPC wrapper, then **re-reads from the backend**;
stores also re-read on the events above. No token ever passes the IPC: gh and
glab keep theirs.
Components use design tokens (`src/styles/tokens.css`), never hard-coded
colors/durations/easings.

## Build, run, test

`just` lists recipes. The Tauri CLI is the local dev-dependency (`pnpm tauri …`).

```
just dev              # Vite + Tauri, hot reload (common.just)
just build            # .app + APFS .dmg (common.just)
just dev-signed       # debug .app, stable signing identity, no hot reload (common.just)
just build-signed     # release .app, stable signing identity (common.just)
just install-signed   # signed release into /Applications, no Finder window (common.just)
just ci               # what CI runs: rust, then frontend (common.just)
just rust             # rustfmt, clippy -D warnings, cargo test (common.just)
just frontend         # prettier, eslint, vitest, build (common.just)
just check            # cargo check --workspace (common.just)
just test-core        # argus-core only (common.just)
just format           # rustfmt + prettier (common.just)
just version NEW      # set the version across the manifests (common.just)
just version-check    # versions in sync across manifests (common.just)
just test-integration # real GitHub API, read-only; needs gh + INTEGRATION_TOKEN
just coverage         # cargo-llvm-cov + Vitest v8 (not in `just ci`)
just icons            # regenerate app icons from the SVG (needs rsvg-convert)
```

Recipes marked `common.just` come from rust-base (see above).

### Signed builds on macOS

Argus keeps no token of its own (gh and glab hold the logins), so there is no
keychain prompt any more. `just dev` ad-hoc signs the binary with a new
signature on every rebuild, so macOS forgets privacy grants (folder access)
each run; a properly signed release keeps them.

- `just dev-signed`, `just build-signed` and `just install-signed` sign with a
  stable identity from the gitignored `src-tauri/tauri.dev-signed.conf.json`
  (only `bundle.macOS.signingIdentity`), so grants persist and Argus runs as
  a real .app. `dev-signed` has no hot reload; keep `just dev` for fast
  iteration.
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
  double routing paths to `tests/fixtures/gitlab/*.json`, reached through
  a fake `glab` (`tests/support/glab.rs`, `FakeGlab`: bash + curl,
  forwards `glab api` to the double adding glab's own `PRIVATE-TOKEN`,
  prints `--include` output, exit 1 + body on stderr for a non-2xx, `dial
  tcp` on stderr when it cannot connect, answers `auth status --all
  --output json` from a `hosts` file (`set_hosts`), `log_out`/`log_in`
  toggle the token; logs argument lines and `env-token=<n>`; `auth_calls`);
  a fake `gh` for accounts (`tests/support/gh.rs`, `FakeGh`: `auth status
  --json hosts` from a file, `text_only` plays a gh without `--json`);
  `tests/accounts.rs` runs both through `Core::with_cli` (hosts, caching
  rules, several GitLab hosts polled on their own, switching off);
  `FakeGlab::shared()` for tests that never read its log; Unix-only); `tests/support`'s
  `Platform`, a scripted provider that logs every call as read or write
  (the guarantee tests); a real temp git repo as the walk's ground truth
  (`history_git.rs`). Review worktrees: a platform repo, a bare copy as
  the clone's remote (a local path, no network) and a clone without the
  head (`git/worktree_tests.rs`, `ai/tests.rs` `review_repo`); skills from
  a fake Claude config dir (`JobContext::claude_dir`,
  `Core::set_claude_dir`), never the real `~/.claude`.
- **Integration tests** (`crates/argus-core/tests/github_integration.rs`):
  the GitHub provider against the real API, read-only, on the public repo
  `RobbieMinderhoud/rust-base` (merged PRs #1 and #2). All `#[ignore]`d;
  `just test-integration` runs only that binary with `--ignored` (never
  `--workspace -- --ignored`: that would also run measurements such as
  `big_review_timings`). Needs the `gh` CLI and `INTEGRATION_TOKEN` (a token
  that can read public repos; the test hands it to `gh` as `GH_TOKEN`); unset,
  each test prints "skipped" and passes. Locally:
  `INTEGRATION_TOKEN=$(gh auth token) just test-integration`. CI runs it
  nightly (`integration.yml`, secret `INTEGRATION_TOKEN`). No writes, ever.
  **GitLab has no real-API test:** the instance is internal and unreachable
  from GitHub runners, so GitLab stays on its local double
  (`gitlab_provider.rs`).
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
  bodies echoing the token through every provider call (GitLab: the fake
  glab's login, a `glpat-` and a 64-hex OAuth-shaped one, echoed on stderr
  too, and glab's own output without a response);
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
  of truth; `sync-version.sh` propagates it (`--check` detects drift).
- `ci.yml`: frontend and Rust checks (`just frontend` / `just rust`), a
  Windows job (`cargo test -p argus-core`, scope set in `.github/rust-base.json`) and the `ci-ok` gate; on push to
  `main` the release follows. `pr-checks.yml`: changelog entry and release
  label. `build.yml` (dispatch-only, run by the release): .dmg and NSIS
  installers. `integration.yml`: nightly `just test-integration`.
- **Rust checks run on ubuntu** (the shared job installs the Tauri system
  packages). Code and tests must pass on Linux as well as macOS: gate anything
  that needs a desktop session or macOS (keychain, Finder, `open`) behind
  `cfg`, and keep tests off the real keychain.
- The first release ships the `Cargo.toml` version.
- `RELEASE_DEPLOY_KEY` (optional secret) lets the release commit through a
  protected main; without it the job falls back to `GITHUB_TOKEN`.
  `INTEGRATION_TOKEN` (secret) is for the nightly integration run.
