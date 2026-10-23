<p align="center">
  <img src="icons/source/app-icon-transparent-128.png" alt="Loom" width="96" height="96">
</p>

<h1 align="center">Loom</h1>

<p align="center">
  A fast, minimal code editor with an integrated terminal and Git, written in Rust.
</p>

<p align="center">
  <a href="https://github.com/lius-new/loom/actions/workflows/ci.yml"><img src="https://github.com/lius-new/loom/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="License: MIT OR Apache-2.0"></a>
</p>

Loom is a desktop editor for Windows, macOS and Linux built on
[lgui](https://github.com/lius-new/lgui), a Rust GUI library that renders with
Skia. It aims to stay small and responsive while covering the daily loop of
editing, searching, running commands and working with Git, without leaving the
window.

> [!NOTE]
> Loom is in early development (version 0.1). Expect rough edges, and expect
> settings and behavior to change between releases.

![Loom with Source Control, a diff of the working tree, the file tree and a terminal](docs/images/screenshot.png)

## Features

- **Editing** — grapheme-aware UTF-8 editing (CJK, combining marks, emoji),
  preserved LF/CRLF line endings, per-tab undo history, split panes, IME input
  and an optional Vim mode. Files up to 64 MiB open on a background
  worker; long lines and large files stay responsive.
- **Markdown** — live reading and side previews of unsaved buffers, shared with
  hover/completion/signature documentation; logical copy, outline, code, tables
  and bounded images. See [Markdown reading](docs/markdown.md).
- **Sessions** — tabs, panes, selections and terminals come back exactly as you
  left them after a restart.
- **Workspace search** — search every folder with case, whole-word, regex and
  include/exclude globs; respects `.gitignore` and searches unsaved buffers.
- **Integrated terminal** — PowerShell, Command Prompt, Git Bash, bash, zsh,
  fish or any custom shell, in tabs and splits that follow your working
  directory across `cd` and restarts.
- **Git** — staging, commits, fetch/pull/push, branches, tags, stashes,
  history, single-file diffs and worktrees, refreshed in the background
  across every repository in the workspace.
- **Customizable** — light, dark and high-contrast themes, a Zed-style
  `keymap.json` with contexts and key sequences, per-project
  `.loom/settings.json`, interface scaling from 80% to 200% and Material file
  icons.

## Installation

Download a package for your platform from the
[Releases](https://github.com/lius-new/loom/releases) page.

| Platform | Packages |
| --- | --- |
| Windows x64 | `…-windows-x86_64-setup.exe` (per-user installer, no admin rights) or `…-portable.zip` |
| macOS 11+ (Apple Silicon and Intel) | `…-macos-universal.dmg` or `.zip` |
| Linux x64 | `.deb`, `.AppImage`, or `.tar.gz` (run `./install.sh` to install for the current user) |

Each release includes a `SHA256SUMS` file per platform.

- **Windows**: packages are not code-signed yet, so SmartScreen warns on first
  run. A Git runtime is included.
- **macOS**: until the app is notarized, Gatekeeper blocks the first launch.
  Control-click Loom.app and choose **Open**, or run
  `xattr -dr com.apple.quarantine /Applications/Loom.app`. Loom uses the
  system `git`.
- **Linux**: requires glibc 2.39 or newer (for example Ubuntu 24.04, Fedora 40)
  and an X11 or Wayland session. Loom uses the system `git`.

All packages provide a `loom` command that opens files and folders:
`loom .`, `loom path/to/file`. See [Packaging](scripts/package/README.md) for
what each package installs.

## Building from source

Loom builds with the Rust toolchain pinned in
[`rust-toolchain.toml`](rust-toolchain.toml) (currently 1.97.1); `rustup`
installs it automatically. Install Node.js 20+ and the platform C/C++ SDK as
well; the shared scripts prepare CMake and Skia privately.

```sh
git clone https://github.com/lius-new/loom.git
cd loom
node scripts/build/cargo.mjs build --release --locked
node scripts/build/stage-resources.mjs target/release
node scripts/build/cargo.mjs run --release --locked
```

For a development run, prepare resources with `node scripts/build/stage-resources.mjs`
before `node scripts/build/cargo.mjs run --locked`. It stages both syntax resources
and the language-server catalog beside the executable; Cargo alone does not copy them.

On Debian or Ubuntu, install the native libraries first:

```sh
sudo apt install build-essential \
    libfreetype-dev libfontconfig-dev \
    libegl1-mesa-dev libgl1-mesa-dev libgles2-mesa-dev \
    libwayland-dev libxkbcommon-dev
```

Pinned CMake and prebuilt Skia archives are downloaded and SHA-256 verified during
the first build; no system CMake or manual Skia download is required. See
[Building Loom](docs/building.md) for offline builds, tests, CI and packaging.

## Documentation

- [Markdown reading](docs/markdown.md) and [architecture](docs/architecture/markdown.md)

- [Editing](docs/editing.md) — mouse, keyboard shortcuts, files, sessions and large files
- [Language packages](docs/languages.md) — independent, demand-loaded packages and user installation
- [Workspace search](docs/search.md)
- [Terminal](docs/terminal.md)
- [Git](docs/git.md)
- [Settings and key bindings](docs/settings.md)
- [Updates](docs/updates.md)
- [Building Loom](docs/building.md)

## Contributing

Bug reports and pull requests are welcome on
[GitHub](https://github.com/lius-new/loom/issues). Before opening a pull
request, run the same checks as CI:

```sh
cargo fmt --all -- --check
node scripts/build/cargo.mjs test --locked --workspace
node scripts/architecture/check.mjs
```

and Clippy with the flags in [`ci.yml`](.github/workflows/ci.yml). The editing
core in [`crates/editor-core`](crates/editor-core/README.md) has no GUI
dependencies and is the easiest place to start.

## License

Loom is licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution
intentionally submitted for inclusion in Loom shall be dual licensed as above,
without any additional terms or conditions.

Loom includes third-party code (the vt100 parser, parts of Zed's keymap
design and, on Windows, Git); see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
The Material Icon Theme icons carry their MIT license and attribution in
[`assets/icon-themes/material`](assets/icon-themes/material).
