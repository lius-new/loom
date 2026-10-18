# Settings and key bindings

## Configuration files

User settings (`settings.json`), key bindings (`keymap.json`) and the saved
session (`workspace.json`) live in the user configuration folder:

| Platform | Folder |
| --- | --- |
| Windows | `%APPDATA%\Loom` |
| macOS | `~/Library/Application Support/Loom` |
| Linux | `$XDG_CONFIG_HOME/loom`, or `~/.config/loom` |

A workspace folder can override settings in `.loom/settings.json`; the
`workspace::OpenProjectSettingsFile` action creates and opens it. Appearance
preferences, such as the theme and the interface scale, can only be set in the
user `settings.json`.

## Interface size

Settings → Appearance → **Interface scale** resizes the entire app, including
tabs, sidebars, menus, icons and controls. Use **−** / **+** to choose 80%–200%
in 10 percentage point steps, relative to the operating system's display scale.
**Reset** restores automatic scaling. Changes apply immediately and persist
across restarts. Editor and terminal font sizes can still be adjusted separately.

**Ctrl + +** (or **Ctrl + =**) and **Ctrl + −** adjust the terminal font size
while the terminal has focus, and the editor font size otherwise. **Ctrl + 0**
resets that font's size. On macOS, use Command instead of Ctrl.

The same setting is available in the user `settings.json` as `"ui_scale": 120`.
Remove it or set it to `null` to restore automatic scaling. Like other appearance
preferences, it cannot be overridden by project settings.

## File associations

Loom picks a file's language from the file itself: its name and extension. To
treat other files as a language, add ordered rules to `settings.json` (or to a
workspace folder's `.loom/settings.json`):

```jsonc
"file_associations": [
  { "pattern": "**/*.custom", "language": "rust" },
  { "pattern": "scripts/*.tool", "language": "javascript" }
]
```

`language` is a language name or alias such as `rust`, `csharp`, `javascript`
or `typescript`. Rules are tried in order and the first match wins. A project's
rules apply to files in that folder and come before your own, and both come
before the built-in recognition. Changes apply to open files immediately, and a
rename picks the language for the new name.

Patterns use a small set of wildcards:

- `*` matches any run of characters within one path segment, `?` matches one.
- `**` as a whole segment matches any number of folders, including none.
- A pattern without `/` matches the file name at any depth (`*.custom`).
- A pattern with `/` matches the path below the workspace folder, from its
  start (`docs/**/*.txt`). Files outside every workspace folder only match
  patterns without `/`.
- Case is ignored on Windows and macOS and respected elsewhere.
- `[ ]`, `{ }` and `\` are not supported; a rule using them is skipped.

A rule with an unknown language or an unsupported pattern is skipped and
reported; the other rules still apply.

## Language servers

Loom talks to language servers (rust-analyzer for Rust) for the files you open.
A server that is not installed is skipped quietly; nothing else changes. To
change how a server runs, or to turn it off, add entries to `settings.json`:

```jsonc
"language_servers": [
  {
    "server": "rust-analyzer",
    "command": "C:\tools\rust-analyzer.exe",   // another program
    "args": [],                                   // replaces the default arguments
    "initialization_options": { "checkOnSave": false },
    "enabled": true
  }
]
```

Only `server` is required; everything else keeps the built-in value. The last
entry for a server wins. A server runs once per project folder: the folder
holding the nearest (or, for Rust, outermost) project file such as
`Cargo.toml`, never above the open workspace folder. It stops a minute after
its last file is closed, and a crashed server is restarted after 1, 2 and 4
seconds before being reported as failed.

This setting is only read from your own `settings.json`: a project's
`.loom/settings.json` cannot choose which program Loom runs.

## Key bindings

Shortcuts come from a Zed-style keymap. The **Keymap** page (Ctrl + K Ctrl + S,
or Settings → Keyboard → Open Keymap) opens in its own tab and lists every
action with its bindings; filter it by action, keystroke or context. Its
**Edit keymap.json** button opens your `keymap.json` in the
[configuration folder](#configuration-files). Saved changes apply immediately;
entries with mistakes are reported and skipped.

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

The default bindings are in [`assets/keymaps/default.json`](../assets/keymaps/default.json).
`LOOM_LOG_KEYS=1` in a debug build prints how each keystroke resolved.
