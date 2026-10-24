# Git

Press `Ctrl+Shift+G` (or `Command+Shift+G`) to open Source Control. Loom discovers
repositories for every workspace root and direct nested repository, then keeps
branch, upstream, ahead/behind, staged, working-tree, untracked, rename and
conflict state refreshed in the background. The title bar, status bar, file
tree, editor gutter and Source Control drawer consume the same immutable
repository snapshot.

## Source Control

The Source Control drawer supports staging and unstaging, commit, fetch, pull
and push. The Git backend also provides typed operations for hunk patches,
branches, tags, history, blame, stash, worktrees, remotes, merge/rebase/
cherry-pick recovery, submodules, LFS and sparse checkout. Git operations run
outside the UI thread, are serialized per repository, classify common errors,
and redact credentials from diagnostic output.

Source Control has a file filter and repository summary above the changes.
Each file row shows its file-type icon and Git status. Check a file to stage it,
or uncheck its staged row to unstage it. File rows still open diffs and drag into
editor panes; the staging checkbox never starts a drag. Conflicts retain their
explicit resolution actions.

The commit composer stays at the bottom for both short and long change lists.
It accepts multiline text: Enter inserts a newline and Ctrl+Enter (Command+Enter
on macOS) commits. Amend is available beside the message label; the split
button's menu offers **Commit and Push** plus the existing commit options.
Commit is disabled when the message or staged changes are missing, conflicts
remain, or Git is busy. Amend can reuse the previous message without staged
changes. A repository with no changes says **Working tree clean.**

## Git panel

The Git panel's tab bar appears at the bottom of the active Git diff
(Commit/Changes), and under any open file while the Source Control drawer is
open. With the drawer closed, ordinary file editing does not show it, and
Settings and other pages never do or reserve space for it. Switching back restores its expansion state, tab
and height. It is expanded by default. Click **History**,
**Stashes**, **Worktrees** or **Tags** to expand its content. Clicking the active
tab again, or the arrow at the right, collapses the content while keeping the
tab bar available. `Ctrl+Shift+H` (or `Command+Shift+H`) also expands or collapses
the panel. There is no title-bar toggle for the Git panel.

When the panel is at least 900px wide, its only tab is **History** and a
navigator fills the left side. An icon rail along its edge switches between
branches (Local and Remote folders), tags and stashes, each with its count; only
the chosen kind is listed. Choose a
branch or tag to show its commits; the current branch returns to HEAD. Choose a
stash to list its files in the detail pane, where **Apply**, **Pop** and **Drop**
live. Right-click a tag to push or delete it. A branch checked out in another
worktree shows a folder icon whose menu opens the worktree, adds it to the
workspace or deletes it. Narrower panels keep the four tabs.

The detail pane lists a commit's or stash's changed files as a folder tree. Folders
that hold a single folder join into one row, fold shut when clicked, and stay
pinned at the top while their files scroll.

Each group title has a **+** button: **BRANCHES** opens the branch picker, where a
new name creates a branch and **New Worktree** checks it out in its own folder;
**TAGS** tags the selected commit (HEAD when none is selected); **STASHES**
stashes the current changes. Branch names with `/` group into folders that fold
shut, and a remote's branches sit under its name.

Drag the expanded panel's top edge to resize it. Collapsing and reopening keeps
the chosen height and tab. Hovering a commit or tag shows its details card, and
the rows keep the same actions and context menus as before. History loads only
while the History tab is expanded.

In narrow editor regions, the tabs scroll horizontally using the wheel or the
header's arrow buttons. The collapse button stays fixed at the right edge.

## Diffs

Drag a changed file from Source Control onto an editor pane to open its
single-file diff, keeping the staged or working-tree comparison shown in the
list. Dropping near a pane edge creates a split; dropping on a tab strip inserts
the diff at the marker. Releasing outside the editor cancels the drag. Clicking
a changed file continues to open All Changes at that file's section.

## Worktrees

The branch picker groups worktrees beneath their checked-out branch. Branch rows
show the worktree count and expand or collapse the directory list. Each worktree
has its own path and actions; choosing one adds it to the workspace and selects
its repository. The existing folders stay available in the repository switcher.

In the branch picker, select an available branch and choose **New Worktree**.
Typing a new branch name also offers this action. The dialog shows the branch and
destination folder, defaulting to a sibling such as `../loom-feature-x`;
**Browse…** changes its parent folder. Creation adds the worktree to the workspace
and selects its repository.

The **Worktrees** tab, shown in narrow panels, lists each worktree with its branch,
current marker and lock status. Hover a row to open, add or delete it. Deletion asks for confirmation; modified or untracked files
require a separate force-delete confirmation. The main, current and locked
worktrees cannot be deleted from this section.

## Open buffers

Open editor buffers are reconciled after Git operations: clean files reload
automatically, while unsaved buffers are never overwritten and are marked when
their disk version changes.

## Git runtime

The Windows packages contain a managed Git runtime (MinGit) under
`runtime/git`, with a SHA-256 manifest that Loom verifies before using it.
Loom prefers that runtime and falls back to the `git` on PATH when no managed
runtime is present, which is always the case for the macOS and Linux packages.
Advanced users and development builds can select a custom runtime through
`GitRuntimeManager`.
