# Settings and key bindings

## Configuration files

User settings (`settings.json`), key bindings (`keymap.json`) and the saved
session (`workspace.json`) live in the user configuration folder:

| Platform | Folder |
| --- | --- |
| Windows | `%APPDATA%\Loom` |
| macOS | `~/Library/Application Support/Loom` |
| Linux | `$XDG_CONFIG_HOME/loom`, or `~/.config/loom` |

On Windows, all persistent user data uses `%APPDATA%\Loom` (Roaming), including
downloaded language resources under `language-packs\.managed`. Loom does not split
user data between the Roaming and Local AppData folders.

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

Loom ships no language servers and does not look for them on your PATH. The
servers it can run are the ones in the catalog published with each release
(`lsp-servers/lsp-servers.toml` beside the application), which describes how to
start each server and which languages it serves, plus servers you register
yourself. A language can have several servers; they all run for a file, and their
diagnostics are shown together. A language that no installed server serves just
has no language-server features.

`language_servers` is an object keyed by server name.

```jsonc
"language_servers": {
  // A catalogued server: change how it behaves, never which program it is.
  "rust-analyzer": {
    "enabled": true,
    "install": "ask",                              // ask | auto | never
    "args": [],                                    // replaces the default arguments
    "initialization_options": { "checkOnSave": false },
    "settings": { "rust-analyzer": { "check": { "command": "clippy" } } },
    "env": { "RA_LOG": "info" }
  },
  // A server of your own: a full definition. Its `command` must be an absolute
  // path; Loom never searches PATH.
  "my-server": {
    "command": "C:\\tools\\my-server.exe",
    "args": ["--stdio"],
    "languages": [{ "language": "foo", "language_id": "foo" }],
    "root": { "markers": ["foo.toml"], "strategy": "nearest", "fallback": "workspace" },
    "priority": 10
  }
}
```

For a catalogued server only `enabled`, `install`, `args`, `initialization_options`,
`settings`, `env` and `priority` may be given; its `command`, `languages`, `root` and
`display_name` are the catalog's. To run another program, register it under a new
name. An entry with a mistake is reported and skipped; the others still apply.

A server runs once per project folder, found with the project files its definition
names (the nearest or the outermost one), never above the open workspace folder. It
stops a minute after its last file is closed, and a crashed server is restarted
after 1, 2 and 4 seconds before being reported as failed.

`install` says what Loom does when a file needs a catalogued server whose files are
not on this computer yet: `ask` (the default) reports the server as not installed
and waits until you install it from the language panel, `auto` downloads and
installs it right away, and `never` leaves it alone. A server you register needs no
installation. Downloads are checked against the catalog's SHA-256 and the server is
started once before it is activated.

`settings` is what the server gets when it asks for its configuration
(`workspace/configuration`). Changing it tells the running server; it is not
restarted. Changing `args`, `initialization_options` or `env` starts a new instance.

### Languages

```jsonc
"languages": {
  "typescript": {
    "language_servers": ["biome", "typescript-language-server", "..."],
    "formatter": "biome",
    "format_on_save": true,
    "code_actions_on_save": ["source.organizeImports"]
  }
}
```

`language_servers` chooses which servers a language uses and in what order (the
first is preferred); `"..."` stands for the servers not named, in their usual
order, and a list without it leaves the others out. `formatter` names the server
that formats (by default the first that can). `format_on_save` and
`code_actions_on_save` run before the file is written; a server that has not
answered within a second is skipped and the file is saved anyway.

### Behaviour

```jsonc
"lsp": {
  "hover_delay_ms": 300,             // how long the pointer rests before hover opens
  "completion": {
    "auto": true,                    // open the list while typing; false = only the key
    "accept_on": "enter_and_tab",    // enter_and_tab | enter | tab
    "words_fallback": true           // complete words of the file without a server
  },
  "inlay_hints": false,
  "semantic_highlighting": "combined",   // combined | off
  "max_document_bytes": 8388608          // larger files are not sent to servers
}
```

These settings are only read from your own `settings.json`: a project's
`.loom/settings.json` cannot choose which program Loom runs.

### Keys

| Key | Action |
| --- | --- |
| Ctrl+Space | Show completions |
| Ctrl+Shift+Space | Signature help |
| Ctrl+K Ctrl+I | Hover |
| F12 / Ctrl+F12 / Ctrl+Shift+F12 / Alt+F12 | Definition / implementation / type definition / declaration |
| Shift+F12 | References |
| F2 | Rename |
| Ctrl+. | Code actions |
| Shift+Alt+F | Format file |
| Ctrl+K Ctrl+F | Format selection |
| F8 / Shift+F8 | Next / previous problem |
| Ctrl+Shift+O / Ctrl+T | Symbols in the file / in the workspace |
| Alt+Left / Alt+Right | Back / forward |
| Shift+Alt+Right / Left | Grow / shrink the selection by syntax |

In Vim's normal mode: `K` hover, `gd` definition, `gD` declaration, `gy` type
definition, `gi` implementation, `gr` references.

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
