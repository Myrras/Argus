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
  reviewed (the menu also works with the arrow keys). The window has the same floating-panel look as Podium and Iris,
  with a resizable sidebar and dark, light and retro themes.
- Review rounds: Argus remembers where you last reviewed each item. Items that
  need another look say why (new commits, the author replied, or your review
  was re-requested) and how many of your threads nobody has touched since.
  Threads you started or replied in count as yours: they are shown first,
  tracked for rounds (a reply after your last comment there counts, and so
  does one you had not seen yet when you posted from Argus) and judged by
  the AI. "Mark round reviewed" moves the item to Waiting on
  author, with a confirmation; items you approved drop off the list.
- A detail view per review, on one scrolling page like a platform's "Files
  changed" tab combined with its conversation, better organised: its title,
  a link that opens it in the browser, status and round, then a
  Conversation section with the comments on the review as a whole (yours
  first, the others folded to one line), then every changed file with its
  threads. A file tree (folders or a flat list, with a filter) starts with
  the Conversation and shows per file whether it was added, changed, removed
  or renamed, its added and removed lines, its threads (yours highlighted)
  and a Viewed checkbox; clicking an entry scrolls to it. Each file shows a
  syntax-highlighted diff, unified or side by side (remembered), with
  unchanged code folded until you expand it. Threads sit under their line,
  yours open and the others folded; outdated threads are listed at the top
  of their file with the code they were made on, and are never hidden.
  Threads on files the diff does not show are listed under the
  Conversation. Each thread on code can expand its then/now history in
  place. Viewed marks are kept per review and clear when the file changes
  again, with "N of M files viewed" above the page. From the second round,
  "Since my last review" shows only what changed since you last marked a
  round reviewed. Binary files and very large diffs stay folded ("Load
  anyway"), and files load as you scroll to them. `j`/`k` move through the
  Conversation and the files, `v` marks the selected file viewed.
- Commenting from the diff: hover a line to see its `+` (only on lines the
  platform accepts a comment on), click it to write a comment under the
  line, or shift-click or drag to comment on a range. From the keyboard,
  `Tab` into a diff, pick a line with `↑`/`↓` and press `Enter`. The comment box takes
  markdown and has a Preview. "Add to review" keeps it as a pending comment
  (shown under its line, editable and deletable, and kept when Argus
  restarts); "Comment now" posts just that one. The review header counts
  your pending comments with "Post all": what posts becomes a normal
  thread, and a comment the platform refuses stays pending with its reason,
  ready to edit and retry, without holding up the rest. Nothing is ever
  posted until you press a button.
- Replying: every thread has a Reply button with the same comment box. If
  the platform refuses, the reason shows and your text stays.
- Approving: an Approve button in the review header, separate from posting,
  asks first and names the commit you are approving. If the review got new
  commits since you looked, the approval is refused so nothing unseen gets
  approved; pending comments are not posted with it (the dialog says how
  many). An approved review leaves the lists. Argus never merges.
- Comments render as markdown: headings, lists, task lists, tables, quotes,
  links (they open in your browser, relative ones such as GitLab
  attachments resolved against the review) and code blocks highlighted by language
  (jq included) with a copy button. Images show as links; HTML in a comment
  shows as text and never runs. Suggested changes (GitHub's and GitLab's
  `suggestion` blocks) show as a small diff of the lines they replace and
  the suggested lines, or just the suggested lines when the original ones
  are unavailable. Argus never applies them.
- Reviews refresh in the background every 3 minutes, or on demand with the
  refresh button or `⇧R`, with the time of the last refresh in the title bar.
  The sidebar's Providers section shows each provider's state and says when
  `gh` is missing or logged out or GitLab is not set up (or off because its
  URL is not https), and a provider that fails to refresh is reported there
  without blocking the other. A manual refresh while one is running waits for
  it instead of running alongside. Other
  failures show as a notification. GitHub also works when Argus is launched
  from Finder or the Dock.
- Settings, opened with `⌘,` (the app menu on macOS, the gear in the title
  bar elsewhere), in tabs: General (dark, light or retro theme, reduced
  motion, the refresh interval), GitHub (on or off, host and account),
  GitLab (URL and token; the token goes to the keychain only, Settings shows
  just whether one is set, and it can be removed; the URL's capitals or a
  `:443` do not lose it), AI (the AI analysis on
  or off, and a local clone folder per repository), and Shortcuts. Switches apply at once, text fields have their
  own Save.
- Keyboard: `↑`/`↓` to move through the list, `Enter` to open, `⇧R` (or
  `⌘R`) to refresh, `1`–`4` for the tabs, `j`/`k`/`v` in a review, `c` to
  comment on the line you last clicked, `⌘↩` to submit a comment and `Esc`
  to cancel it (Undo in the notification brings the text back), `⌘,` for
  Settings. Every shortcut except `Esc` can be
  rebound in Settings → Shortcuts.
- Thread history: every thread on code shows the code it was left on and
  that same code now, side by side (stacked in a narrow card), highlighted,
  with the changed lines marked and the commented lines edged. Your own
  threads show it straight away; other reviewers' threads open it with
  "Then/now history" or `h` on the thread you last clicked. A line above
  says what happened: moved from one line to another, renamed, touched by N
  commits or untouched since your comment, removed or deleted in which
  commit, or that Argus could not locate the lines (never a guessed line).
  The comments become a timeline with the commits that touched the lines
  and resolves in between, so you see at a glance whether someone replied
  before changing anything; long comments fold. Outdated threads stay fully
  readable, and comments on the review as a whole get the timeline too.
- AI analysis: with it on in Settings, your local agent (Claude Code or
  auggie) reviews each new item and each new round on its own. The review
  header shows where it stands (not started, queued, running, done with the
  number of findings, or failed with the reason), with Run again, Retry or
  Cancel, and the list marks items whose analysis is running. Findings sit
  inline in the code under their line, coloured by severity and labelled
  AI, or at the top of their file when the line is not in the diff. The AI
  panel at the right (`a` shows or hides it) lists them by severity with
  the ones from earlier rounds marked, and "Look here yourself": what the
  AI could not check on its own, to tick off per round. Select findings
  (click or `x`), edit their text with a preview, or dismiss them; "Post
  selected" posts them as line comments, links each posted one, and keeps a
  refused one selected with its reason without holding up the rest.
  Nothing is posted until you press it. In later rounds each of your
  threads shows whether the AI thinks it was addressed, partly addressed or
  untouched, with its reasoning; you can set your own call or clear it. The
  label stays in Argus and never changes the thread on GitHub or GitLab.
  The AI never repeats a point already raised in the review, by anyone,
  open or resolved, in English or Dutch: it reads every thread (yours,
  other reviewers', the author's, on code or on the review as a whole) and
  every finding you posted, and names the threads each point overlaps.
  Findings and checklist items that repeat an existing comment are set
  aside under "Skipped as already raised (N, k high)", open by itself when
  a high one is among them, where you can still read them, jump to the
  comment they repeat, or show one anyway; nothing is dropped. A new point
  on code someone already commented on is kept and tagged "related to
  bob's comment", and a high-severity finding is never set aside only
  because the AI called it related.
  With the AI analysis off, none of this shows and everything else works
  as before.
- Polling is lighter: Argus asks GitHub and GitLab only for what changed since
  the last refresh, so unchanged reviews barely count against your rate limit.
- GitHub threads now show as resolved when they are resolved on GitHub.
- When GitHub or GitLab rate-limits Argus, it pauses that provider and shows
  "rate limited until HH:MM" instead of retrying in vain.
- A hung `gh` call or an unresponsive GitLab instance no longer stalls
  refreshing: calls time out and the other reviews still update.
- Thread history and file contents now read from your local clone when one is
  set for the repository: faster and exact. Commits you haven't fetched still
  come from GitHub/GitLab, and Argus never changes or fetches the clone.
- Review a merge request without the mouse: `n`/`p` move through threads and
  AI findings in page order, `j`/`k` still move between files. The focused
  item gets a focus ring. `Enter` or `t` opens or folds the focused thread,
  `r` replies, `h` shows then/now, `c` comments on its line; on a finding `x`
  selects it, `e` edits it and `d` dismisses it.
- Command palette (⌘K / Ctrl+K): every action with its shortcut, plus Go to
  review… and Go to file…. Approve… and Post all… still ask before anything
  is sent.
- `?` shows all keyboard shortcuts and where they work.
- Refresh is now ⇧R (or ⌘R / Ctrl+R): plain `r` replies to the focused
  thread. If you set your own Refresh key, it still works.
- Dialogs now focus their contents when they open, keep `Tab` inside, and
  give the focus back when they close.

- The AI agent runs sandboxed: in its own temporary folder with your clone
  read-only beside it, so the branch under review cannot configure it with
  its own agent settings, hooks or instructions. Claude can only read the
  review's files and the clone, not your home folder or other repositories,
  and anything shaped like a token or private key is removed from findings
  before they are saved, including more token shapes. Claude cannot read
  the clone's `.git` (nested repositories included), `.env` files,
  `.netrc`/`.npmrc`/`.pypirc`, private keys (`*.pem`, `*.key`) or local app
  secrets (`appsettings.Local.json`, `appsettings.<env>.Local.json`,
  `appsettings.Secrets.json`, `local.settings.json`, `.dev.vars`,
  `secrets.json`/`.yml`); the committed `appsettings.json` and
  `appsettings.<env>.json` stay readable. Auggie runs keep only its read tools: Argus
  removes every other tool Auggie lists, including MCP tools, and does not
  start the run when Auggie cannot list its tools.
- AI findings post on the code the AI actually read; a finding from a later
  round's "since last review" diff on a removed line is refused instead of
  landing on another line, with a hint to re-run the AI pass on the full
  diff. Re-running the AI pass drops stale verdicts,
  never offers an already posted finding again, and keeps your own verdict
  label on a thread in later rounds.
- Posting or approving when new commits arrived in between is refused and
  shows the new commits right away. A multi-line comment is refused rather
  than moved when lines inside it changed.
- Quitting Argus stops running AI passes and removes their temporary files;
  files left by a crash are cleaned up on the next start. It no longer waits
  for a refresh that is still talking to GitHub or GitLab.
- A self-hosted GitLab may use plain http (for example behind a VPN). Argus
  warns that the token then travels unencrypted: a note under the URL in
  Settings and an "Unencrypted" badge in the Providers section. A token
  stored for https is never sent over plain http: switching the URL to http
  asks for the token again, and Settings says why. A plain-http GitLab set
  up before keeps its token; if the keychain could not be read when Argus
  first moved it, the move is tried again on the next start.
- On a pull request too large for GitHub to find its merge base, removed
  lines take no comments (the file says why; the `c` key and posting refuse
  them as well, so nothing lands on the wrong line), unless a local clone supplies
  the merge base: then comments and AI findings on them post as shown. The
  clone needs the base branch's latest commit; fetch it when the file still
  says removed lines take no comments, then press "Check again" on that note
  (or open another file) to pick up the merge base.
- Opening Argus a second time brings the running window to the front. If
  Argus cannot open its data at startup it says so, naming the folder.
- The macOS menu bar has a Window menu: Minimize (⌘M), Zoom, full screen and
  Close Window (⌘W).
- GitHub pull request conversation comments and review summaries now show in
  the Conversation section and count for rounds and the AI pass. Replies
  you post from Argus stay in their thread. Quote replies count when they
  quote whole lines or a selection of one (one short line like "> ok" is no
  reply), and only the author's very next real answer after your comment
  counts as a reply to you: not `/retest`, `@dependabot rebase` or a remark
  to someone else (a mention inside code, like `@Nullable`, is no remark),
  and not a later comment after the author answered someone else.

## [0.1.0] - 2026-10-16

### Added

- First Argus build: a desktop window with a placeholder start screen and a
  dark/light theme toggle. Reviewing merge requests and pull requests comes in
  later releases.
- Argus app icon and logo: a violet eye, shown in the title bar and on the
  start screen.
