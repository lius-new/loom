# Packaging

Node.js scripts (no npm dependencies, Node 20+) that build Loom's
distributable packages.

```bash
node scripts/package/package.mjs all
```

| Step | Purpose |
| --- | --- |
| `setup` | Download and verify the packaging toolchain and inputs (Inno Setup, MinGit) into `.packaging/tools` |
| `compile` | `cargo build --release --locked` |
| `package` | Stage the application folder with the managed Git runtime, smoke-test it, write the portable zip |
| `bundle` | Build the installer from the staged folder and write `SHA256SUMS` |

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
  tools/       innosetup-<version>/, mingit-<version>/
  windows/     stage/
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

## Other platforms

macOS and Linux are not implemented yet (`--platform macos|linux` reports
this); their release workflows are placeholders.
