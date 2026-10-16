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
  author and age.
- Review rounds: Argus remembers where you last reviewed each item. Items that
  need another look say why (new commits, the author replied, or your review
  was re-requested) and how many of your threads nobody has touched since.
  "Mark round reviewed" moves the item to Waiting on author; items you approved
  drop off the list.
- A detail pane per review: its title, a link that opens it in the browser,
  status and round, and every comment thread, yours first and the others
  folded under their own heading. Each thread shows its file and line (or
  that it is on the merge request as a whole), whether it is resolved or
  outdated, and all its comments in order. Outdated threads are never hidden.
- Reviews refresh in the background every 3 minutes, or on demand with the
  refresh button or `r`, with the time of the last refresh in the title bar.
  A status bar says when `gh` is missing or logged out or GitLab is not set
  up, and a provider that fails to refresh is reported without blocking the
  other. GitHub also works when Argus is launched from Finder or the Dock.
- Settings: GitHub host and account, GitLab URL and token (the token goes to
  the keychain only; Settings shows just whether one is set), the refresh
  interval, and AI analysis on or off (saved now, used in a later release).
- Keyboard: `↑`/`↓` to move through the list, `Enter` to open, `r` to
  refresh, `1`–`4` for the tabs, `⌘,` for Settings. Light and dark theme.

## [0.1.0] - 2026-10-16

### Added

- First Argus build: a desktop window with a placeholder start screen and a
  dark/light theme toggle. Reviewing merge requests and pull requests comes in
  later releases.
- Argus app icon and logo: a violet eye, shown in the title bar and on the
  start screen.
