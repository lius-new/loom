# Loom

A minimal text editor written in Rust, using the
[lgui](https://github.com/lius-new/lgui) GUI library.

## Project layout

```
loom/
├── assets/       # application and file-type icons
├── Cargo.toml    # package manifest + lgui dependency
└── src/          # application, editor, model, terminal, and UI modules
```

## Features

- UTF-8 text editing with grapheme-safe movement and deletion (including Chinese,
  combining marks and Emoji); LF and CRLF line endings are preserved.
- Click a file in the tree to open or reactivate its tab. Each document keeps its
  own selection, scroll position and undo/redo history.
- Click to place the caret, drag to select, Shift-click to extend the selection,
  double-click to select a word, and triple-click or click a line number to select
  a line. Dragging line numbers selects whole lines. Holding a drag beyond the
  viewport scrolls continuously.
- Right-click for Undo, Redo, Cut, Copy, Paste and Select All. Clipboard failures
  leave the source text intact.
- Mouse wheel scrolls vertically; Shift-wheel scrolls horizontally.
- Chinese IME preedit text is displayed at the caret, with the native candidate
  window anchored there. Only committed input changes the document.

### Git worktrees

The branch picker groups worktrees beneath their checked-out branch. Branch rows
show the worktree count and expand or collapse the directory list. Each worktree
has its own path and actions; choosing one adds it to the workspace and selects
its repository. The existing folders stay available in the repository switcher.

Source Control shows a **WORKTREES** section when a repository has multiple
worktrees, with the branch, current worktree and lock status. Hover a row to open,
add or delete it. Deletion asks for confirmation; modified or untracked files
require a separate force-delete confirmation. The main, current and locked
worktrees cannot be deleted from this section.

In the branch picker, select an available branch and choose **New Worktree**.
Typing a new branch name also offers this action. The dialog shows the branch and
destination folder, defaulting to a sibling such as `../loom-feature-x`;
**Browse…** changes its parent folder. Creation adds the worktree to the workspace
and selects its repository.

### Editing shortcuts

| Shortcut | Action |
| --- | --- |
| Arrow keys | Move by character or line; vertical movement remembers the desired column |
| Ctrl + Left / Right | Move by word |
| Home / End | Move to line start / end |
| Ctrl + Home / End | Move to document start / end |
| Page Up / Page Down | Move by one viewport |
| Shift + any movement above | Extend selection from its anchor |
| Ctrl + A | Select all |
| Ctrl + C / X / V | Copy / cut / paste |
| Ctrl + Z / Y | Undo / redo (Ctrl + Shift + Z also redoes) |
| Backspace / Delete | Delete the selection or previous / next character |
| Ctrl + Backspace / Delete | Delete the selection or previous / next word |
| Enter | Insert a newline with the current indentation |
| Tab / Shift + Tab | Indent / outdent; a selection applies to all selected lines |
| Esc | Close the editor menu, or clear the selection |
| Ctrl + S | Save the active file |

Indentation uses two spaces; existing tabs display at four-column tab stops.
Typing replaces the selection. Consecutive typing/deletion is grouped for undo;
movement, paste, indentation, saving and focus changes separate undo groups.
Editor shortcuts apply while the editor has focus; the terminal keeps its own
keyboard handling. On macOS, Command takes the place of Ctrl.

### Terminal sessions

On Windows, Loom offers installed PowerShell, Command Prompt and Bash (with
Git Bash preferred). On Linux and macOS, the system default follows `$SHELL`,
then the account's login shell, then `/bin/sh`.

Settings → Terminal lets you add, edit and remove custom terminals by name,
executable and launch arguments, and choose a default. Unavailable custom
terminals keep their configuration and are disabled until their path is fixed.
An unavailable saved default is kept, and new terminals use the first available
shell until its path is fixed. Restored terminals whose shell is unavailable
start a fallback shell with a notice, and launch their own shell again the next
time they start after it is repaired. Deleting the default terminal resets the default.

PowerShell, Command Prompt, Bash, zsh, fish and POSIX shells can report their
working directory to Loom. Custom commands and startup-file arguments take
precedence over automatic integration and may provide their own directory reports.
Each terminal remembers its latest directory after `cd` and restores it when
the application restarts. New tabs and splits inherit the current terminal's
directory. If a saved directory no longer exists, Loom falls back to the first
workspace folder, then the application's working directory. Shell profiles
and custom prompts are loaded without modifying user configuration files.

### Customizing key bindings

Shortcuts come from a Zed-style keymap. The **Keymap** page (Ctrl + K Ctrl + S,
or Settings → Keyboard → Open Keymap) opens in its own tab and lists every
action with its bindings; filter it by action, keystroke or context. Its
**Edit keymap.json** button opens your `keymap.json` (`%APPDATA%\Loom` on
Windows). Saved changes apply immediately; entries with mistakes are reported
and skipped.

```jsonc
[
  {
    "context": "Editor",                     // where the bindings apply
    "bindings": {
      "alt-up": "editor::MoveToBeginning",   // keystrokes → action
      "secondary-y": null,                   // null disables a default binding
      "secondary-k secondary-d": ["pane::ActivateItem", 0] // sequences, arguments
    }
  }
]
```

- **Keystrokes** join modifiers with `-`: `ctrl`, `alt`, `shift`, `cmd`, and
  `secondary` (Command on macOS, Ctrl elsewhere). Separate a sequence with spaces.
- **Contexts**: `Workspace` (always), `Editor`, `Terminal`, `Input`, `Menu`,
  `CloseConfirmation`, `CloneRepository`. Combine them with `&&`, `||`, `!` and
  `>` (ancestor), and test attributes such as `extension == rs` or `os == windows`.
- **Precedence**: a binding in a more specific context wins (Editor over
  Workspace); within one context, later bindings — yours — win.

`LOOM_LOG_KEYS=1` in a debug build prints how each keystroke resolved. The design
is documented in [KEYMAP_PLAN.md](KEYMAP_PLAN.md).

## File icons

The entire workspace tree uses a focused subset of the open-source Material
Icon Theme, including closed folders, open folders, and file types. Exact
filenames such as `Cargo.toml` and `Dockerfile` take priority over extensions;
compound extensions are matched longest-first, and unknown files use a generic
document icon. Attribution, source revision, and the upstream MIT license are
stored with the SVG assets under
`assets/icon-themes/material/`.

## Git integration

Press `Ctrl+Shift+G` (or `Command+Shift+G`) to open Source Control. Loom discovers
repositories for every workspace root and direct nested repository, then keeps
branch, upstream, ahead/behind, staged, working-tree, untracked, rename and
conflict state refreshed in the background. The title bar, status bar, file
tree, editor gutter and Source Control drawer consume the same immutable
repository snapshot.

The Source Control drawer supports staging and unstaging, commit, fetch, pull
and push. The Git backend also provides typed operations for hunk patches,
branches, tags, history, blame, stash, worktrees, remotes, merge/rebase/
cherry-pick recovery, submodules, LFS and sparse checkout. Git operations run
outside the UI thread, are serialized per repository, classify common errors,
and redact credentials from diagnostic output.

Official packages contain a managed Git runtime with a SHA-256 manifest. Loom
prefers that runtime and falls back to system Git when a managed runtime is not
present. Advanced users and development builds can select a custom runtime
through `GitRuntimeManager`. Open editor buffers are reconciled after Git
operations: clean files reload automatically, while unsaved buffers are never
overwritten and are marked when their disk version changes.

## Build & run

```sh
cargo build --release
cargo run --release
```

The project follows the `main` branch of the lgui repository. `Cargo.lock`
pins the exact lgui commit used by a build. Run `cargo update -p lgui` when you
want to update that pinned commit.

## Continuous integration and releases

GitHub Actions checks formatting and Clippy on Linux, then tests and builds Loom
on Ubuntu x64, Windows x64, and macOS ARM64. Successful runs expose packaged
binaries as workflow artifacts.

Each platform has its own release workflow. `release-windows.yml` validates
that the requested version matches `Cargo.toml`, builds the per-user installer
and portable zip with the scripts in [`scripts/package`](scripts/package/README.md),
and publishes them to the GitHub release. Push a version tag such as `v0.1.0`,
or run the workflow manually with `0.1.0`. The macOS and Linux workflows are
placeholders until those packages are implemented.

Build the Windows packages locally with:

```sh
node scripts/package/package.mjs all
```

`lgui` uses Skia for rendering and Winit for the window. The renderer is the
**software** Skia backend (`renderer-skia-software`), which presents via
`softbuffer`. `skia-safe` downloads a prebuilt Skia archive automatically at
build time.

## System dependencies (Debian / Ubuntu)

```sh
sudo apt install build-essential \
    libfreetype-dev libfontconfig-dev \
    libegl1-mesa-dev libgl1-mesa-dev libgles2-mesa-dev \
    libwayland-dev libxkbcommon-dev
```

A running X11 or Wayland desktop session is required to open the window.
(On WSL2, enable WSLg and install the packages above inside the distro.)
