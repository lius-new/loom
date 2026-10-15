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

Source Control keeps file changes in the upper area. **WORKTREES**, **STASHES**
and **COMMITS** start collapsed in a separate area anchored at the bottom.
The commit input follows short change lists and stays below the scrollable
changes area for long lists. Expanding the bottom sections scrolls their
contents independently while preserving room for changes above, including
when the repository has no changes.

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

Source Control shows a **WORKTREES** section when a repository has multiple
worktrees, with the branch, current worktree and lock status. Hover a row to open,
add or delete it. Deletion asks for confirmation; modified or untracked files
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
