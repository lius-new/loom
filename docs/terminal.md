# Terminal

## Shells

On Windows, Loom offers installed PowerShell, Command Prompt and Bash (with
Git Bash preferred). On Linux and macOS, the system default follows `$SHELL`,
then the account's login shell, then `/bin/sh`.

## Custom terminals

Settings → Terminal lets you add, edit and remove custom terminals by name,
executable and launch arguments, and choose a default. Unavailable custom
terminals keep their configuration and are disabled until their path is fixed.
An unavailable saved default is kept, and new terminals use the first available
shell until its path is fixed. Restored terminals whose shell is unavailable
start a fallback shell with a notice, and launch their own shell again the next
time they start after it is repaired. Deleting the default terminal resets the default.

## Working directories

PowerShell, Command Prompt, Bash, zsh, fish and POSIX shells can report their
working directory to Loom. Custom commands and startup-file arguments take
precedence over automatic integration and may provide their own directory reports.
Each terminal remembers its latest directory after `cd` and restores it when
the application restarts. New tabs and splits inherit the current terminal's
directory. If a saved directory no longer exists, Loom falls back to the first
workspace folder, then the application's working directory. Shell profiles
and custom prompts are loaded without modifying user configuration files.

## Font size

**Ctrl + +** (or **Ctrl + =**) and **Ctrl + −** adjust the terminal font size
while the terminal has focus. **Ctrl + 0** resets it. On macOS, use Command
instead of Ctrl.

## Scrollback

Terminal panes with history show a faint vertical scrollbar that brightens when
the pointer is in the pane or the thumb is dragged. Drag the thumb to browse
history; it stays bright while dragging outside the pane. Each split terminal
has its own scrollbar. Like the editor, it overlays the text at the right edge
without reserving width or changing the terminal's columns.
