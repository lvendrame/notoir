# 28 — Git & Source Control

## Purpose

Define Git integration: architecture, repository detection, status and decorations, the Source Control panel, commit,
branches, network operations (fetch/pull/push/sync), clone/init, file history, compare with HEAD, and the opt-in "Sync
Notes" workflow. Git never freezes the UI and never pushes without explicit consent.

## Architecture (AD-7)

`notoir-git` exposes a request/response API executed on workers; it holds no UI state:

```rust
pub trait GitBackend: Send + Sync {
    fn discover(&self, path: &Path) -> Result<Option<RepoInfo>, GitError>;
    fn status(&self, repo: &RepoInfo, opts: StatusOpts) -> Result<RepoStatus, GitError>;
    fn diff_file(&self, repo: &RepoInfo, path: &Path, against: DiffBase) -> Result<FileDiff, GitError>;
    fn read_blob(&self, repo: &RepoInfo, path: &Path, rev: &str) -> Result<Vec<u8>, GitError>;
    fn stage(&self, repo: &RepoInfo, paths: &[PathBuf]) -> Result<(), GitError>;
    fn unstage(&self, repo: &RepoInfo, paths: &[PathBuf]) -> Result<(), GitError>;
    fn discard(&self, repo: &RepoInfo, paths: &[PathBuf]) -> Result<(), GitError>;
    fn commit(&self, repo: &RepoInfo, msg: &str, opts: CommitOpts) -> Result<CommitId, GitError>;
    fn branches(&self, repo: &RepoInfo) -> Result<Vec<BranchInfo>, GitError>;
    fn checkout(&self, repo: &RepoInfo, branch: &str, opts: CheckoutOpts) -> Result<(), GitError>;
    fn create_branch(&self, repo: &RepoInfo, name: &str, from: &str, checkout: bool) -> Result<(), GitError>;
    fn log_file(&self, repo: &RepoInfo, path: &Path, limit: usize) -> Result<Vec<CommitSummary>, GitError>;
    fn init(&self, path: &Path) -> Result<RepoInfo, GitError>;
}

pub trait GitRemote: Send + Sync {           // system git CLI, streamed output, cancellable
    fn fetch(&self, repo: &RepoInfo, opts: FetchOpts, out: OutputSink, cancel: CancellationToken) -> Result<(), GitError>;
    fn pull(&self, repo: &RepoInfo, mode: PullMode /* FfOnly | Rebase | Merge */, out: OutputSink, cancel: CancellationToken) -> Result<PullResult, GitError>;
    fn push(&self, repo: &RepoInfo, opts: PushOpts, out: OutputSink, cancel: CancellationToken) -> Result<(), GitError>;
    fn clone(&self, url: &str, dest: &Path, out: OutputSink, cancel: CancellationToken) -> Result<RepoInfo, GitError>;
}
```

- Local operations: `git2` (vendored libgit2) on the blocking pool.
- Network operations: system `git` (`git.path` or found on PATH) as a child process with `GIT_TERMINAL_PROMPT=0` and
  `GCM_INTERACTIVE=never`-style flags so it never hangs on a hidden prompt; stdout/stderr streamed to the Git Output
  channel; progress parsed for the status bar. If no system git exists, fall back to git2 network ops with credential
  helper + ssh-agent support and show a hint recommending installing Git.
- Authentication failures surface as "Push failed. Authentication required." with [Show Git Output] [Retry] and a link
  to docs on configuring credentials. Notoir does not store Git credentials itself.
- Status refresh: debounced (500 ms) after watcher events in the work tree or `.git/index`/`HEAD`/refs, after saves,
  after Git commands, and on window focus. Uses `StatusOpts { include_untracked: true, recurse_untracked_dirs: true,
  exclude_submodules: true }`; > 10k changes → truncated with a notice.
- Concurrency: a per-repo operation queue; write operations are serialized; status queries are cancellable and
  superseded.
- `index.lock` present → operations show "Another Git process is running" with Retry.

## Repository Detection

On workspace open (`git.autoDetect = true`): discover the repo containing the root (or nested repos up to depth 1 for
monorepo-of-notes setups, `git.scanNestedRepos`). Files outside any repo get no decorations. Settings: `git.enabled`
(default true), `git.autoDetect` (true), `git.autoFetch` (**false**), `git.pullOnOpen` (**false**), `git.autoCommit`
(**false**), `git.autoPush` (**false**, restricted), `git.confirmSync` (true), `git.pullMode` (`ffOnly`).

## Decorations

- Explorer: letter badges + colours (spec 07).
- Tabs: file name tinted by status (optional `git.decorateTabs`, default true).
- Gutter + minimap: line change bars computed by diffing buffer vs index (or HEAD when unstaged view preferred,
  `git.gutterBase = index`), recomputed on a worker after edits (300 ms debounce). Clicking a gutter bar shows an inline
  peek with the original lines and actions: Revert Change, Stage Change (Phase 3 later).
- Status bar: `⎇ main ↑1 ↓2` (ahead/behind upstream from last fetch), `⎇ main*` when dirty work tree; detached HEAD
  shows the short SHA; ongoing merge/rebase shows "MERGING"/"REBASING".

## Source Control Panel

```text
SOURCE CONTROL                              [✓ commit] [⟳ refresh] ⋯
┌───────────────────────────────────────────┐
│ Message (Mod+Enter to commit)            │
│ Added JSON tree viewer                   │
└───────────────────────────────────────────┘
[ Commit ▾ ]                    ← Commit · Commit All · Commit & Push (opt-in) · Amend Last Commit

▾ Staged Changes (1)                         [− unstage all]
    A ideas/new-feature.md
▾ Changes (2)                                [+ stage all]
    M docs/design.md
    M README.md
▾ Merge Conflicts (0)                        (hidden when empty)

Sync Changes ↑2 ↓1
```

- Click a row: opens a diff (index/HEAD vs work tree) via spec 27. Double-click: opens the file.
- Row hover actions: Open File, Stage `+` / Unstage `−`, Discard (`undo-2` icon).
- Discard always confirms with the file name and "This cannot be undone" (untracked files are moved to trash, not
  deleted). Discard All shows the number of affected files and a confirmation button in `danger` colour.
- Commit with nothing staged: "There are no staged changes. Stage all changes and commit? [Stage All & Commit] [Cancel]"
  (`git.smartCommit = ask`).
- Commit message validation: empty → error; a subtle ruler at 72 chars on body lines.
- Commit message draft persisted per repo in the session.
- View list as tree or list (`⋯` menu).
- Context menu on rows: Open File, Open Changes, Stage, Unstage, Discard Changes…, Add to .gitignore, Reveal in Explorer,
  Copy Path, File History.

## Branches

- Status bar branch click / Git → Branches › Switch Branch…: picker listing local branches, remote branches, recent;
  "Create Branch…" and "Create Branch From…" items at top.
- Switching with dirty documents in Notoir: prompt to Save All first (buffers are not files Git can see). With a dirty
  work tree that conflicts: Git's error is shown with options "Stash & Switch" (Phase 3 later; stash is listed as later
  scope) / Cancel. Open documents whose files changed are reloaded (clean) or flagged (dirty) per spec 10.

## Network Operations

- Fetch, Pull (`ffOnly` default; when not fast-forwardable: "Your branch and 'origin/main' have diverged. [Pull with
  Rebase] [Pull with Merge] [Cancel]"), Push (sets upstream automatically on first push after confirmation), Sync
  (pull then push; confirms once per workspace when `git.confirmSync = true`).
- All run in the background with status bar progress and cancel; never modal.
- No operation runs automatically unless the user enabled the corresponding setting; `git.autoFetch` when enabled runs
  every `git.autoFetchPeriod` (default 300 s) and never pulls.

## Clone & Init

- Git → Clone Repository…: URL input (+ recent URLs), destination folder picker, progress in status bar; on success
  "Open cloned repository? [Open] [Open in New Window]".
- Git → Initialize Repository: on the workspace root (confirm), creates the repo; offers creating a `.gitignore`.

## File History & Compare with HEAD

- File History (Explorer/tab/palette): list of commits touching the file (author, date, message, short SHA) in the
  bottom panel or a picker; selecting one opens a diff of that revision vs its parent; "Compare with Current" option;
  "Open File at Revision" (read-only virtual doc).
- Compare with HEAD: diff of HEAD blob vs current buffer (spec 27 `GitHead`).

## Sync Notes (opt-in workflow)

Command "Git: Sync Notes" (shown when `notes.sync.enabled = true`, workspace trusted):

```text
1. Save all dirty documents in the workspace (prompts for Untitled ones are skipped; they are not part of the repo)
2. Stage all changes (respecting .gitignore)
3. Commit with message template notes.sync.commitMessage (default "Notes sync {{date}} {{time}} from {{hostname}}")
4. Fetch; if behind: pull --rebase only when no local/remote conflict is predicted (dry-run merge-tree check);
   otherwise stop and open Source Control with an explanation
5. Push
```

- Each step reports in Git Output; any failure stops the pipeline without destructive recovery (no reset, no force push,
  no auto-resolve).
- Optional triggers (all default off): sync on workspace open, sync every N minutes, sync on window close; enabling any
  trigger requires a confirmation dialog describing that it will commit and push automatically.
- A conflict during rebase aborts the rebase (`git rebase --abort`) and informs the user; their local commit remains.

## Later Scope (not in Phase 3)

Stash, blame, merge UI/conflict editor, cherry-pick, rebase tools, partial-line staging, submodules management,
signed commits UI, Git LFS UI.

## Test Cases

```gherkin
Feature: Repository detection

  Scenario: Status decorations
    Given a workspace that is a Git repo with modified "README.md" and untracked "idea.md"
    When it is opened
    Then the Explorer shows "M" on README.md and "U" on idea.md
    And the status bar shows the current branch

  Scenario: No auto fetch by default
    Given a fresh install
    When a Git workspace is open for 10 minutes
    Then no network Git command has run

Feature: Commit

  Scenario: Commit staged changes
    Given "README.md" is staged and the message is "Update readme"
    When the user presses Mod+Enter in the message box
    Then a commit "Update readme" is created containing README.md
    And the Staged Changes section is empty

  Scenario: Smart commit prompt
    Given nothing is staged and two files are modified
    When the user clicks Commit
    Then the user is asked to stage all and commit

  Scenario: Discard requires confirmation
    When the user clicks Discard on "README.md"
    Then a confirmation names "README.md" and warns it cannot be undone

Feature: Network operations

  Scenario: Push never freezes the UI
    Given a push takes 10 seconds
    When the user pushes
    Then the status bar shows progress and the editor stays responsive

  Scenario: Authentication failure
    Given the remote requires credentials that are not configured
    When the user pushes
    Then an error notification says "Push failed." with "Authentication required."
    And Git Output contains git's stderr

  Scenario: Diverged branch
    Given local and remote have diverged
    When the user pulls
    Then the user is offered Pull with Rebase, Pull with Merge, or Cancel

Feature: Sync Notes

  Scenario: Sync stops on predicted conflict
    Given Sync Notes is enabled and the remote changed the same lines as a local edit
    When the user runs "Sync Notes"
    Then local changes are committed
    And no pull/rebase or push is performed
    And Source Control opens with an explanation

  Scenario: Git checkout reloads clean open files
    Given "a.md" is open and clean
    When the user switches to a branch where "a.md" differs
    Then the editor shows the branch's version of "a.md"
```
