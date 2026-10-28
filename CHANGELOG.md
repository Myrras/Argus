# Changelog

All notable **functional** (user-facing) changes to this project are documented
here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Internal refactors, tooling, and chores are intentionally omitted. Every pull
request adds its entry under `## [Unreleased]`; the version number is assigned
when it merges.

## [Unreleased]

### Added

- The review cockpit: one list of the GitHub pull requests and GitLab merge
  requests where you are a requested reviewer or have commented, read through
  the `gh` CLI and a GitLab token kept in the system keychain. Four tabs with
  counts: Needs review, Needs re-review, Waiting on author, and Closed without
  my approval. Your review state on GitHub or GitLab decides the tab; items you
  approved leave the list and come back when the approval is reset. Right-click
  an item to open it, copy its link, refresh it or submit your review.
- Review rounds: Argus remembers where you last reviewed each item and says
  why it needs another look (new commits, the author replied, your review was
  re-requested), with how many of your threads nobody has touched since.
- A detail view per review on one scrolling page: the conversation, then every
  changed file with its threads. A file tree (folders or flat, with a filter)
  shows per file its status, added and removed lines, your open threads and a
  Viewed checkbox. Syntax-highlighted diffs, unified or side by side, with
  unchanged code folded; "Since my last review" shows only what changed since
  your last round. Outdated threads are never hidden, and a thread on code can
  show its then/now history. Binary files and very large diffs stay folded.
- Commenting from the diff (click a line, or shift-click or drag for a range),
  replying, and resolving or reopening threads, on GitHub and GitLab. Comments
  can stay pending ("Add to review", kept across restarts) and go out with
  "Post all". A refused post stays editable with the platform's reason. Nothing
  is posted until you press a button.
- Submit review (header, right-click menu, palette or ⇧S): Approve or Request
  changes (on GitHub also Comment), with an optional summary and your pending
  comments. It is refused when new commits arrived since you looked. On GitLab
  Argus adds you as a reviewer first when you need to request changes (GitLab
  17.2 or later). Argus never merges.
- Comments render as markdown, with highlighted code blocks and suggested
  changes shown as a small diff (never applied). HTML never runs.
- AI analysis (off by default; Settings → AI): your local agent (Claude Code
  or auggie) reviews each new item and round. Findings sit inline under their
  line, with an AI panel (`a`) that lists them by severity and a "Look here
  yourself" checklist. You select, edit or dismiss findings and post them
  yourself. In later rounds each of your threads shows whether the AI thinks it
  was addressed. The AI never repeats a point already raised in the review;
  such points are set aside and still readable.
- With a local clone set per repository, history and file contents read from
  the clone, files open in your editor from the file tree (`o`, Settings →
  Editors), and the AI pass reviews the merge request's own code in a temporary
  worktree and follows the project's review rules (`CLAUDE.md`, `AGENTS.md`,
  Cursor and Copilot rules) and review skill, chosen per repository in
  Settings → AI. The clone itself is never changed.
- Keyboard and navigation: `↑`/`↓` and `Enter` in the list, `1`–`4` for the
  tabs, `j`/`k`/`v` between files, `n`/`p` between threads, `r`, `c`, `h`, `s`
  on a thread, ⌃/⌄ between unresolved threads, a command palette (⌘K), `?` for
  all shortcuts, every one except `Esc` rebindable in Settings → Shortcuts.
- Settings (`⌘,`): theme (dark, light, retro), reduced motion, refresh
  interval, GitHub host and account, GitLab URL and token (stored in the
  keychain only), AI, editors and shortcuts.
- Background refresh every 3 minutes, on demand with ⇧R, or for a single
  review with ⌥R. A Providers section shows each provider's state; a failing
  provider does not block the other.
- Argus brings the running window to the front when you open it a second time,
  and the macOS menu bar has a Window menu.
- A self-hosted GitLab may use plain http (for example behind a VPN); Argus
  warns that the token then travels unencrypted and never sends a token stored
  for https over http.
- CI now uses the shared workflows from ci-workflows. Checks, release and
  installer builds are unchanged for contributors; a nightly run against the
  real GitHub API is added.

### Changed

- Rate limits and failures are gentler: Argus asks only for what changed since
  the last refresh, pauses a rate-limited provider ("rate limited until
  HH:MM") and times out a hung `gh` call or GitLab instance without stalling
  the other reviews.
- A clearer review header, with icon buttons for the AI pass, the diff layout,
  the file list and shortcut help, and a local clone chip.
- Review rounds follow your review state on the platform: Submit review
  replaces "Mark round reviewed".
- Closing Argus stops running AI passes and removes their temporary files.
- The AI agent runs sandboxed: it cannot read your home folder, the clone's
  `.git`, `.env` files, keys or local app secrets, and anything shaped like a
  token is removed from findings before they are saved.

### Fixed

- Posting or approving after new commits arrived is refused and shows them at
  once; a multi-line comment is refused rather than moved when lines inside it
  changed.
- AI findings post on the code the AI actually read, never on another line.
- GitHub threads show as resolved when they are resolved on GitHub, and pull
  request conversation comments and review summaries count for rounds and the
  AI pass.
- On a pull request too large for GitHub to find its merge base, removed lines
  take no comments instead of landing on the wrong line.

## [0.1.1] - 2026-10-28

### Changed

- Every pull request now needs exactly one release label (`patch`, `minor` or
  `major`). A `Release label` check fails the PR without it, and the label sets
  the version bump on merge.

## [0.1.0] - 2026-10-16

### Added

- First Argus build: a desktop window with a placeholder start screen and a
  dark/light theme toggle. Reviewing merge requests and pull requests comes in
  later releases.
- Argus app icon and logo: a violet eye, shown in the title bar and on the
  start screen.
