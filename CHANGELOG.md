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
  my approval. Each item shows its platform, repository and number, title,
  author and age; right-click one to open it, copy its link or mark its round
  reviewed. The window has the same floating-panel look as Podium and Iris,
  with a resizable sidebar and dark, light and retro themes.
- Review rounds: Argus remembers where you last reviewed each item. Items that
  need another look say why (new commits, the author replied, or your review
  was re-requested) and how many of your threads nobody has touched since.
  "Mark round reviewed" moves the item to Waiting on author, with a
  confirmation; items you approved drop off the list.
- A detail view per review, with a Threads and a Files tab (the changed files
  arrive in a later release): its title, a link that opens it in the browser,
  status and round, and every comment thread, yours first and the others
  folded under their own heading. Each thread shows its file and line (or
  that it is on the merge request as a whole), whether it is resolved or
  outdated, and all its comments in order. Outdated threads are never hidden.
- Reviews refresh in the background every 3 minutes, or on demand with the
  refresh button or `r`, with the time of the last refresh in the title bar.
  The sidebar's Providers section shows each provider's state and says when
  `gh` is missing or logged out or GitLab is not set up, and a provider that
  fails to refresh is reported there without blocking the other. Other
  failures show as a notification. GitHub also works when Argus is launched
  from Finder or the Dock.
- Settings, opened with `⌘,` (the app menu on macOS, the gear in the title
  bar elsewhere), in tabs: General (dark, light or retro theme, reduced
  motion, the refresh interval), GitHub (on or off, host and account),
  GitLab (URL and token; the token goes to the keychain only, Settings shows
  just whether one is set, and it can be removed), AI (analysis on or off,
  saved now and used in a later release, and a local clone folder per
  repository), and Shortcuts. Switches apply at once, text fields have their
  own Save.
- Keyboard: `↑`/`↓` to move through the list, `Enter` to open, `r` (or
  `⌘R`) to refresh, `1`–`4` for the tabs, `⌘,` for Settings. Every shortcut
  can be rebound in Settings → Shortcuts.

## [0.1.0] - 2026-10-16

### Added

- First Argus build: a desktop window with a placeholder start screen and a
  dark/light theme toggle. Reviewing merge requests and pull requests comes in
  later releases.
- Argus app icon and logo: a violet eye, shown in the title bar and on the
  start screen.
