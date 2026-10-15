# Packaging

Node.js scripts (no npm dependencies, Node 20+) that build Loom's
distributable packages.

```bash
node scripts/package/package.mjs all                    # current OS
node scripts/package/package.mjs all --platform linux   # explicit
```

| Step | Purpose |
| --- | --- |
| `setup` | Download and verify the packaging toolchain and inputs into `.packaging/tools` (Windows: Inno Setup, MinGit; Linux: appimagetool and its runtime) |
| `compile` | `cargo build --release --locked` |
| `package` | Stage the application folder, check it, write the portable archive (Windows zip with the managed Git runtime, Linux tar.gz) |
| `bundle` | Build the installers from the staged folder (Windows setup, Linux .deb and AppImage) and write `SHA256SUMS` |

Every step can run on its own; each checks that the previous one has run.
`--expect-version x.y.z` fails early when `Cargo.toml` has a different version
(release workflows pass the tag).

## Layout

All downloads, tools, staging folders and artifacts live in the git-ignored
`.packaging/` directory. Nothing is installed system-wide; delete the folder to
start clean. Inno Setup is installed with its own portable mode
(`/PORTABLE=1`), which writes no registry entries, shortcuts or uninstaller.

```
.packaging/
  downloads/   verified archives (reused while their SHA-256 matches)
  tools/       innosetup-<version>/, mingit-<version>/, appimagetool-<version>/
  windows/     stage/
  linux/       stage/, deb/, appimage/
  dist/        final artifacts
```

## Windows

The installer is built with Inno Setup and modelled on Zed's Windows installer
(`windows/installer.iss`): modern wizard that follows the system light/dark
mode, English or Simplified Chinese chosen from the system language, a
destination page, an additional-tasks page and a finish page that can launch
Loom.

- `Loom-<version>-windows-x86_64-setup.exe`: per-user install into
  `%LOCALAPPDATA%\Programs\Loom`, no elevation. Tasks, all selected by default:
  desktop shortcut, Start menu shortcut, "Open with Loom" for files, "Open with
  Loom" for folders (folder, folder background, drive), and `loom` on the
  user's PATH (`bin\loom.cmd`). Unchecking a task on upgrade removes what it
  created earlier. Silent install: `/VERYSILENT`; custom folder: `/DIR=<path>`;
  skip tasks: `/MERGETASKS="!desktopicon,!addtopath"`. Uninstalling removes the
  installed files, shortcuts, menu entries and PATH entry, never user data.
- `Loom-<version>-windows-x86_64-portable.zip`: the same application folder.
- Both carry MinGit under `runtime/git` with `MANIFEST.json`, `VERSION`,
  `SOURCE.txt` and Git's `LICENSE.txt`.
- Packages are not code-signed yet, so SmartScreen will warn on first run.

`Loom.exe <path>...` opens the first folder as the workspace, adds further
folders, and opens files as tabs; the Explorer entries and `loom` rely on it.

To upgrade Inno Setup or MinGit, update the version, URL and SHA-256 together
in `windows/index.mjs`. Never change the installer's `AppId`.

## Known limitations

- Windows 11 top-level context menu: "Open with Loom" only appears under
  "Show more options". The top-level menu needs an `IExplorerCommand`
  component in a signed sparse MSIX package, so it waits until Loom has code
  signing.

## Linux

Built on x86_64 Linux with `tar`, `dpkg-deb` and `dpkg-shlibdeps` (Debian
or Ubuntu: `dpkg-dev`). appimagetool and the AppImage type 2 runtime are
pinned downloads; appimagetool runs from its extracted contents, so the build
needs no FUSE, and the runtime is passed explicitly so nothing unpinned is
fetched.

- `Loom-<version>-linux-x86_64.tar.gz`: the application folder. Run
  `./install.sh` to copy it to `~/.local/lib/loom` and add `~/.local/bin/loom`,
  the menu entry and icons for the current user; `install.sh --uninstall`
  removes exactly those. `./Loom` also runs in place.
- `Loom-<version>-linux-x86_64.deb`: installs into `/opt/loom`, with
  `/usr/bin/loom`, the menu entry and hicolor icons. Depends on `git` and the
  libraries `dpkg-shlibdeps` finds; the X11/Wayland libraries winit opens at
  runtime are Recommends. Pre-release versions use `~` so they sort below the
  release.
- `Loom-<version>-linux-x86_64.AppImage`: the same folder under
  `usr/lib/loom`. It does not bundle shared libraries; it relies on the
  desktop libraries every distribution ships.
- Linux packages carry no Git runtime: Loom uses the `git` on PATH.
- `loom <path>...` detaches from the terminal, like `loom.cmd` on Windows.

The release binary requires the glibc of the build machine (the `package`
step prints it); building on ubuntu-24.04 means glibc 2.39 or newer.

To upgrade appimagetool or the runtime, update the version, URL and SHA-256
together in `linux/index.mjs`.

## Other platforms

macOS is not implemented yet (`--platform macos` reports this); its release
workflow is a placeholder.
