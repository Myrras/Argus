# Changelog

All notable **functional** (user-facing) changes to this project are documented
here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Internal refactors, tooling, and chores are intentionally omitted. Every pull
request adds its entry under `## [Unreleased]`; the version number is assigned
when it merges.

## [Unreleased]

## [0.1.0] - 2026-10-16

### Added

- First Argus build: a desktop window with a placeholder start screen and a
  dark/light theme toggle. Reviewing merge requests and pull requests comes in
  later releases.
- Argus app icon and logo: a violet eye, shown in the title bar and on the
  start screen.
- Argus now reads your reviews from GitHub (through the `gh` CLI) and GitLab
  (with a token kept in the system keychain): merge requests and pull
  requests where you are a requested reviewer or have commented, with their
  commits, reviewers and comment threads, including outdated ones.
- Review rounds: Argus remembers where you last reviewed each item and tells
  whether it needs your first review, needs another look (new commits, a
  reply in one of your threads, or a new review request), is waiting on the
  author, or was merged or closed without your approval. Items you approved
  drop off the list.
- Reviews refresh in the background every 3 minutes; settings (GitHub host
  and account, GitLab URL, poll interval, AI on/off, local clones) are saved
  between runs.
