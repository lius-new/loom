# Editing

## Files and tabs

- UTF-8 text editing with grapheme-safe movement and deletion (including Chinese,
  combining marks and Emoji); LF and CRLF line endings are preserved.
- Click a file in the tree to open or reactivate its tab. Each document keeps its
  own selection, scroll position and undo/redo history.
- Open UTF-8 text files up to 64 MiB from the Explorer, file dialog or launch
  arguments. Workspace search has a separate 4 MiB per-file limit.
- On Windows, drag files from another application into an editor pane or its
  tab strip to open them as permanent tabs in that pane. Multiple files open in
  order; existing tabs and unsaved edits are preserved. Folders are ignored.
- Interactive file opens prepare text and indexes on a background worker while
  retaining the current editor. A target tab reserves its icon slot; only opens
  taking more than 300 ms show a loading indicator. Close that tab to cancel;
  choosing another file or existing tab supersedes the pending open.

## Sessions

- Restarting restores every file tab left open, including single-click previews
  and a pending file load, in its saved pane and tab order. Normal window close
  saves the final snapshot before exiting; Save/Don't Save exit decisions keep
  the tab list intact until that snapshot is written. Unreadable files are
  skipped on restore; unsaved text still uses the existing Save/Don't Save prompt.
- Startup restores tab metadata, order, panes and selection before reading any
  file contents. A background worker prepares the selected documents first;
  surviving tabs are filled in place without changing focus. Restoring tabs are
  remembered on exit and cannot edit or save their empty staging buffers.

## Large files

Large documents use plain text rendering and suppress automatic Vim match
highlighting. Long lines are rendered in horizontal viewport slices, without
truncating the file; sparse Unicode indexes keep cursor lookup local. Line
widths, dirty-state checks, syntax colors and text layouts are cached.

## Mouse and input

- Click to place the caret, drag to select, Shift-click to extend the selection,
  double-click to select a word, and triple-click or click a line number to select
  a line. Dragging line numbers selects whole lines. Holding a drag beyond the
  viewport scrolls continuously.
- Right-click for Undo, Redo, Cut, Copy, Paste and Select All. Clipboard failures
  leave the source text intact.
- Mouse wheel scrolls vertically; Shift-wheel scrolls horizontally.
- Chinese IME preedit text is displayed at the caret, with the native candidate
  window anchored there. Only committed input changes the document.

## Keyboard shortcuts

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

Every shortcut can be changed; see [Settings and key bindings](settings.md#key-bindings).

## File icons

The entire workspace tree uses a focused subset of the open-source Material
Icon Theme, including closed folders, open folders, and file types. Exact
filenames such as `Cargo.toml` and `Dockerfile` take priority over extensions;
compound extensions are matched longest-first, and unknown files use a generic
document icon. Attribution, source revision, and the upstream MIT license are
stored with the SVG assets under
[`assets/icon-themes/material/`](../assets/icon-themes/material/).
