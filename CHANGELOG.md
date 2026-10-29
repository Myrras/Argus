# Changelog

All notable **functional** (user-facing) changes to this project are documented
here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Internal refactors, tooling, and chores are intentionally omitted. Every pull
request adds its entry under `## [Unreleased]`; the version number is assigned
when it merges.

## [Unreleased]

### Security

- **Dependency security updates.** Updated vite to 7.3.7 and dependencies to
  their latest minor and patch versions, fixing known vulnerabilities.

## [1.1.0] - 2026-10-29

### Added

- Hide whitespace changes, like GitHub's "Hide whitespace" and GitLab's "Show
  whitespace changes" switch: a button next to the diff layout (or W,
  rebindable, or the command palette) makes lines that changed only in
  indentation, trailing spaces, tabs vs spaces or line endings count as
  unchanged, so they fold away with the rest; highlights inside changed lines
  skip the spaces too. It applies to every review and is remembered; the
  toolbar says "Whitespace hidden" while it is on. A file whose changes are
  all whitespace folds to an "Only whitespace changes" card (with "Show
  changes") and gets a quiet ␣ mark in the file tree, whose +/− counts follow
  the setting. Line numbers, comments, threads and viewed marks stay as they
  are.
- Suggest code changes in comments and replies, like "Insert suggestion" on
  GitHub and GitLab: the comment toolbar's button (or ⌘G / Ctrl+G, rebindable)
  puts a suggestion block at the cursor, filled with the commented lines as
  they are at the head, in the platform's syntax (GitHub `suggestion`, GitLab
  `suggestion:-N+0`). In a reply it applies to the thread's lines. Preview,
  and pending comments, show it as the change it proposes. Inside the block
  Enter keeps the indentation and Tab indents like the file. If the comment's
  lines and the suggestion's drift apart, the composer says so and fixes it in
  one click. Where a suggestion can't go (an old-side line, a GitHub comment
  outside the diff, a review-level or outdated thread, a file without a diff)
  the button stays off and says why.
- Find in the review with Cmd+F (Ctrl+F): a bar under the review's toolbar
  searches the comments, AI findings and both sides of the code (removed lines
  too, and folded unchanged code), in page order, also in files you have not
  scrolled to yet ("Searching 12 of 40 files…"). It shows "3 of 27",
  highlights every match with the current one standing out, and goes there
  with Enter / Shift+Enter, ↑/↓ or Cmd+G / Cmd+Shift+G, wrapping around;
  folded threads, files and unchanged stretches open on the way. Match case,
  Whole word and Regex (an invalid one says why), and Everything / Code only /
  Comments only. Binary, large and patchless files are listed as not searched.
  Cmd+F in the code or in a comment you are writing opens the same bar; Escape
  closes it and puts you back. The keys can be changed in Settings →
  Shortcuts, and the command palette has "Find in review…".
- Comment on any line of a changed file, also unchanged code outside the diff
  (folded or unfolded). GitLab takes it as a normal line comment. GitHub only
  takes line comments inside the diff, so there Argus posts a file comment
  that quotes the lines and links to them at the review's head; Argus shows it
  back at those lines as "on lines 12–18 (outside the diff)". A range stays
  inside one part of the diff: one hunk, or one stretch of unchanged code.
- The comment "+" follows your pointer, beside the line under it (on each side
  in side by side), as on GitHub and GitLab. Its tooltip says you can click or
  drag. While you drag, the lines are highlighted with a clear first and last
  line, a label by the pointer says "Comment on lines 12–18", and a line the
  range can't reach is marked as not allowed. Escape cancels a drag;
  shift-click still extends a comment.
- "Write with AI" reads what you point at: a merge or pull request link of a
  host switched on in Settings → Accounts is fetched (title, description,
  latest comments) and a file path inside one of your local clones or the
  scripts folder is read, then given to the AI as an example. Each request
  shows what was attached ("Attached: MR !4242 · Code_review.md") and what
  was not, with the reason.
- "Write with AI" shows the AI at work: what it is doing, how long it has been
  running, its thinking as it comes in (folded once done) and a Stop button on
  the request. A failed request says why and has a Retry button.
- Open in editor now opens the repository together with the file when an
  editor is set, so Zed, VS Code and the like show the whole project. Turn it
  off in Settings → Editors ("Open the repository with the file"); the system
  default app still opens the file only.
- Set up each repository in one place, Settings → Repositories: its local
  clone (choose the folder, see whether it is there, show it in Finder, clear
  it), its review skill and the language the AI analysis writes its findings,
  checklist and verdicts in. Every repository Argus has seen a review of is
  listed, open ones first; a new one appears after the refresh that found it,
  marked "Not set up". The clone chip and the clone suggestion open the
  repository's card directly.
- Choose the AI review language: Auto (the merge request's own language,
  English when unclear), English, Nederlands or any other, as a default in
  Settings → AI and per repository. Code and identifiers stay as they are; the
  AI chip's tooltip says which language a run used.
- A repository without a local clone set is offered one Argus finds on your
  disk: it looks next to the clones you already use and in `~/code`, `~/src`
  and `~/Developer`, up to three folders deep, and matches on the repository's
  git remote. The review header then says "Found a local clone of … at …. Use
  it?": "Use it" stores it for that repository, "No thanks" means Argus never
  offers one for it again, and × hides the offer until next time. Nothing is
  read from the clone beyond its remotes, and nothing is changed.
- Submit scripts: run your own scripts after you submit a review, for example
  one that ticks the task list in a GitLab merge request's description with
  your own `glab`. Add them in Settings → Scripts, per verdict (approve,
  request changes, comment) and per repository or for all. The Submit review
  dialog lists the ones that fit under "Run after submitting"; they run only
  when ticked there and only once the review is submitted, one after the other
  on your machine, in the repository's local clone (else an empty temporary
  folder). A script gets the review in `ARGUS_*` environment variables and as
  JSON on stdin with your summary, runs with your own tools and logins (Argus
  never passes its token), and is stopped after its timeout. Argus tells you
  when they finish; the Scripts tab shows each script's last run with its
  output.
- Write a submit script with AI: describe what it should do and your AI agent
  (Claude Code or Auggie, as set in Settings → AI) writes it, with a short
  explanation and the verdicts it suits; change the description and regenerate
  until it fits. The agent gets no tools and no access to your files. Nothing
  runs until you save the script (in Argus' own scripts folder, which the tab
  can show in Finder) and tick it when submitting.

### Changed

- Settings → Accounts replaces the GitHub and GitLab tabs. Argus finds every
  host your `gh` and `glab` are logged in to (github.com, GitHub Enterprise,
  gitlab.com, your own GitLab) and polls each of them, several GitLab or
  GitHub hosts at once. Each host has a switch (on by default when you are
  logged in to it; a host you are not logged in to starts off and stays quiet
  until you switch it on), shows who you are logged in as, and, when gh has
  more than one account on a host, which one Argus uses. A host that is not
  logged in shows the exact `gh auth login` / `glab auth login` command to
  copy; "Add an account" gives both commands for a new host, with Check again.
  Argus still never sees or stores a token. Your earlier GitHub host and
  account and your GitLab URL carry over. A host you switch off keeps its
  reviews: they leave the list and come back when you switch it on again. The
  Providers section lists each host in use, and its Settings button opens
  Accounts on the host that needs you.
- Write with AI works in the background and as a conversation. Send your
  description and close the dialog, or Settings, and keep reviewing: the draft
  keeps being written, a toast tells you when it is ready (Open takes you back
  to it), and Settings → Scripts lists your drafts with their status, where
  you can stop, open or discard them. Once a script is written, ask for
  changes: each follow-up keeps your earlier requests as context and gives you
  the whole updated script, with the older versions folded above it. Edit with
  AI on a saved script starts from its current text. Drafts last until Argus
  quits; two are written at a time.
- Argus now uses your glab login for GitLab, the way it uses gh for GitHub,
  and keeps no GitLab token of its own: run `glab auth login --hostname <your
  host>` once. Settings → Accounts shows whether glab is logged in to each
  host and as whom, and gives the exact login command to copy, with Check
  again. Argus asks the CLIs for their logins only at start, when you change
  an account, on Check again and after a call was refused for the login, not
  on every refresh. The Providers section says when glab is missing, not
  logged in or cannot reach the instance. A token you stored in the keychain
  before is no longer read (Argus leaves it there; delete it in Keychain
  Access if you like).
- The review header is now grouped by meaning instead of one run of inline
  badges. Under the title stands what needs you and why: the status with its
  round as one pill, your review state on the platform, why the review is back
  (each reason with its own icon), how many of your threads nobody touched,
  and faint at the end when Argus last saw it. The reference facts — platform
  and number with the buttons to open it or refresh it, the author, when it
  was updated and the local clone — moved to a quieter bar below, with "N
  unresolved threads" and its jump buttons at the right end. In a narrow
  window the actions and the thread navigation drop to their own line.
- The ⌃ and ⌄ beside the review header's unresolved count now loop: ⌄ on the
  last unresolved thread goes to the first, ⌃ on the first goes to the last,
  with a quiet "Wrapped" hint.

### Fixed

- A reply you post now shows in its thread right away. It used to wait for the
  thread's history to be read again, which takes a while after new commits, so
  the reply only appeared after a refresh.
- A thread you resolve no longer keeps counting on its file: the number in the
  file's header now counts open threads only (outdated ones too) and drops as
  soon as you resolve one, and comes back when you reopen it. The Conversation
  header no longer shows a second, total count next to "N open".
- Huge merge and pull requests open and scroll smoothly. A review of hundreds
  of files shows its first diff in about a second; only the files near what
  you are reading keep a live diff, the rest hold their place, so scrolling,
  jumping with j/k, n/p and the find bar stay quick and memory stays flat.
  Large files no longer show everything between the first and last change as
  changed: their hunks fold like any other file, and a file of tens of
  thousands of lines opens in about a second.
- Reopening a review, switching between reviews or restarting Argus no longer
  reads every file from GitHub or GitLab again: file contents and file lists
  are kept on disk (512 MB by default, least recently used out first) and the
  rest of a review's files are fetched quietly in the background. Settings →
  General → Cache shows what it holds, sets the limit and clears it.
- The AI pass and "Write with AI" work with the current auggie (1.x) again.
  Argus reads what your auggie supports from its help, keeps it to reading (no
  shell, no edits, none of your MCP servers, no web) and shows its thinking
  while it writes a script. An auggie Argus can't keep to reading is not run;
  the AI status says "this auggie version can't run read-only" and why. Older
  auggie versions work as before.
- "Write with AI" no longer runs into "the agent took longer than 180s".
  Claude was started with its own agent instructions but without tools, so it
  wrote tool calls as text (trying to open the merge request you linked) and
  never answered; Argus now gives it a script writer's instructions, stops an
  answer that turns into tool calls and asks once more. The time limit is now
  300 seconds.
- Settings is wider and stays inside its window: the title and tabs stay put,
  only the tab's content scrolls, and long paths and commands no longer push
  items past the edge.
- Side by side, opening a folded stretch of unchanged lines now opens it on
  both sides, also when Argus opens it to show a thread, your comment or the
  line you picked, so the two sides stay lined up.
- Comments on a large file of a GitLab merge request: GitLab leaves such a
  file's diff out of its answer, so Argus refused every comment on it with
  "has no text diff to comment on" although the diff was on screen. Argus now
  reads those diffs again in full. When a file really has no diff (binary, or
  only renamed), the refusal says so and what to do instead.
- Windows: submit scripts start as Windows starts commands. A bare name finds
  `tick.cmd` or `tick.exe` on your PATH, `~\…` and `~/…` use your user
  profile folder, a script that can't run there (a `.sh` file) says so, and a
  script that runs past its timeout is stopped together with everything it
  started. A script run without a local clone now starts in its own temporary
  folder instead of the Windows folder.

## [1.0.0] - 2026-10-28

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
