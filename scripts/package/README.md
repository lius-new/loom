# Packaging

Node.js scripts (no npm dependencies, Node 20+) that build Loom's
distributable packages.

```bash
node scripts/package/package.mjs all                    # current OS
node scripts/package/package.mjs all --platform linux   # explicit
```

| Step | Purpose |
| --- | --- |
| `setup` | Download and verify the packaging toolchain and inputs into `.packaging/tools` (Windows: Inno Setup, MinGit; Linux: appimagetool and its runtime; macOS: checks the Xcode tools and adds both Rust targets) |
| `compile` | `cargo build --release --locked` (macOS: both architectures, merged with `lipo`) |
| `package` | Stage the application folder, check it, write the portable archive (Windows zip with the managed Git runtime, Linux tar.gz, macOS signed Loom.app in a zip) |
| `bundle` | Build the installers from the staged folder (Windows setup, Linux .deb and AppImage, macOS .dmg) and write `SHA256SUMS` |
| `manifest` | Write the signed update manifest `update-<platform>.json` (see [Update manifests](#update-manifests)); `all` skips it when no signing key is set |

Every step can run on its own; each checks that the previous one has run.
`--expect-version x.y.z` fails early when `Cargo.toml` has a different version
(release workflows pass the tag). `compile` builds with `LOOM_UPDATES=1`,
which turns on update checks; a plain `cargo build` has them off.

## Layout

Application assembly copies complete prebuilt standard Tree-sitter resources
from LOOM_LANGUAGE_PACKS. Neither compile nor package builds grammar code. Obtain
compatible modules from an external provider and combine them with metadata and
queries following [the resource format](../../docs/languages.md). There is no
language producer project or Loom-specific Rust wrapper.

For local packaging, set LOOM_LANGUAGE_PACKS to the complete collection. For
CI/releases, configure repository variables LOOM_LANGUAGE_PACKS_<TARGET>_URL and
LOOM_LANGUAGE_PACKS_<TARGET>_SHA256 (WINDOWS_X86_64, LINUX_X86_64,
MACOS_UNIVERSAL). fetch-languages.mjs verifies a pinned tar.gz with package folders
at its root, then stages the unmodified modules and queries. Enabled JSON and
Markdown are required for releases. An external resource feed must be supplied;
no provider is built into this repository. CI without a feed exports the tested
host alone, clearly labelled without language resources.

A package may contain standard .wasm files or native modules. Stems prefer the
upstream tree-sitter-<stem>.wasm download, then <stem>.wasm, then the native
platform filename, matching runtime discovery. WASM bytes are copied unchanged
on every platform. For macOS, supply universal native modules; only native
modules are individually signed. The app carries allow-jit for Wasmtime. Missing
or platform-incompatible native files stop assembly.
No Cargo profile or grammar source directory is consulted. Consumer tests:

~~~sh
node --test scripts/package/lib/language-packs.test.mjs scripts/package/fetch-languages.test.mjs
~~~

All downloads, tools, staging folders and artifacts live in the git-ignored
`.packaging/` directory. Nothing is installed system-wide; delete the folder to
start clean. The one exception is `rustup target add` on macOS, which adds the
Rust targets to the toolchain. Inno Setup is installed with its own portable mode
(`/PORTABLE=1`), which writes no registry entries, shortcuts or uninstaller.

```
.packaging/
  downloads/   verified archives (reused while their SHA-256 matches)
  tools/       innosetup-<version>/, mingit-<version>/, appimagetool-<version>/
  windows/     stage/
  linux/       stage/, deb/, appimage/
  macos/       stage/Loom.app, dmg/
  dist/        final artifacts
```

## Update manifests

Loom checks `releases/latest/download/update-<platform>.json`
([Updates](../../docs/updates.md)). The `manifest` step writes it from the
packages in `dist/`: version, release page, and the URL, SHA-256 and size of
each package, signed with Ed25519.

The private key is read from `LOOM_UPDATE_SIGNING_KEY` (PEM text, the
repository secret the release workflows pass) or from the file named by
`LOOM_UPDATE_SIGNING_KEY_FILE`. To create a new key pair:

```bash
node scripts/package/package.mjs keygen
```

It writes `.packaging/update-signing-key.pem` and prints the public key.
Paste the public key into `PUBLIC_KEY` in `src/infrastructure/update.rs`,
store the PEM as the `LOOM_UPDATE_SIGNING_KEY` secret, keep an offline backup
and delete the file. A build trusts only the key it was built with: after a
key change, users of older builds have to update once by hand.

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
  skip tasks: `/MERGETASKS="!desktopicon,!addtopath"`. With `/VERYSILENT`,
  `/relaunch=1` starts Loom when Setup finishes; in-app updates use it. Uninstalling removes the
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

## macOS

Built on macOS with the Xcode Command Line Tools; nothing is downloaded.
`compile` builds `aarch64-apple-darwin` and `x86_64-apple-darwin` with
`MACOSX_DEPLOYMENT_TARGET=11.0` and merges them into one universal binary.

- `Loom-<version>-macos-universal.dmg`: Loom.app next to an Applications link,
  to drag-install.
- `Loom-<version>-macos-universal.zip`: the same Loom.app (made with `ditto`,
  which keeps the signature intact).
- The bundle id is `io.github.lius-new.loom`. `package` fails if the binary
  links anything outside `/usr/lib` and `/System/Library`, so a Homebrew
  library can never sneak in.
- No Git runtime is bundled: Loom uses the `git` on PATH (`/usr/bin/git`
  offers to install the Command Line Tools when they are missing).
- Terminal command: link `Loom.app/Contents/Resources/bin/loom` into PATH, for
  example `ln -s /Applications/Loom.app/Contents/Resources/bin/loom
  /usr/local/bin/loom`. It makes relative paths absolute and starts a new
  instance.

Signing is ad-hoc by default. Gatekeeper then blocks the downloaded app: open
it with Control-click > Open, or run `xattr -dr com.apple.quarantine
/Applications/Loom.app`. Setting `LOOM_MACOS_SIGN_IDENTITY` to a Developer ID
Application identity in the keychain signs with the hardened runtime; adding
`LOOM_MACOS_NOTARY_APPLE_ID`, `LOOM_MACOS_NOTARY_TEAM_ID` and
`LOOM_MACOS_NOTARY_PASSWORD` (an app-specific password) also notarizes and
staples the app and the disk image. `release-macos.yml` lists the matching
repository secrets.

Loom.app declares no document types: opening a file from Finder sends an
Apple Event that Loom does not handle yet, so "Open With" would start Loom
without the file.

### Standard upstream language resources

Language staging accepts collections with tree-sitter.json and the standard
module/query layout described in [docs/languages.md](../../docs/languages.md).
Directory names may retain upstream hyphenated names. It preserves manifests,
modules and declared query paths, including arrays and grammar subdirectories;
no Loom package.json or SOURCE.md is required or generated. Standard LICENSE
files and optional PROVENANCE.md are copied when present. JSON/Markdown release
requirements are checked by grammar name rather than folder name. Legacy
package.json collections remain accepted for migration; SOURCE.md is optional.
