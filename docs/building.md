# Building Loom

## Toolchain

Install Node.js 20+ (CI selects Node 24), Rust and the platform C/C++ SDK.
Rust 1.97.1, Clippy and rustfmt are pinned in
[`rust-toolchain.toml`](../rust-toolchain.toml); CI uses the same compiler.
Windows needs Visual Studio C++ Build Tools and the Windows SDK; macOS needs
Xcode Command Line Tools. Linux system libraries are listed below.

Use the shared build entry point:

```sh
node scripts/build/cargo.mjs build --release --locked
node scripts/build/stage-resources.mjs target/release
node scripts/build/cargo.mjs run --release --locked
node scripts/build/cargo.mjs test --locked --workspace
```

[`scripts/dependencies`](../scripts/dependencies/README.md) automatically downloads
pinned CMake and the matching prebuilt Skia archive, verifies SHA-256 and prepares
only the Cargo child environment. No system CMake, manual Skia download, LLVM
installation or global PATH change is required. `lgui` renders with the software
Skia backend (`renderer-skia-software`) and presents through `softbuffer`.
The managed inputs cover Windows x64, Linux x64 and macOS arm64/x64. CMake is
chosen by host OS/architecture; Skia is chosen by Rust target and resolved features.

To prime the private cache before an offline build:

```sh
node scripts/dependencies/ensure.mjs
node scripts/build/cargo.mjs build --release --locked --offline
```

Cargo crates/toolchains must also be cached for offline use. Bare `cargo` bypasses
the dependency preparer; use the Node entry point for full application builds.
GUI-free core/domain tests and formatting do not prepare these dependencies.
Installed Loom users need none of these build tools.

The project follows the `main` branch of the lgui repository. `Cargo.lock`
pins the exact lgui commit used by a build. Run `cargo update -p lgui` when you
want to update that pinned commit.

The host build and default tests require no language resources or grammar sources.
Loom includes the generic Tree-sitter engine and dynamically loads compatible
standard WASM or native language modules when used. WASM does not require
a platform-specific language library. There is no language producer workspace or
Rust grammar wrapper. Place complete prebuilt language resources beside the
executable or in the user data directory and restart; no editor rebuild is
needed. See [Language resources](languages.md) for the format and release inputs.

For `cargo run` or the shared wrapper's debug run, prepare resources with:

```sh
node scripts/build/stage-resources.mjs
node scripts/build/cargo.mjs run --locked
```

The staging command defaults to `target/debug`. Pass `target/release` for a release
run, or the executable's directory for custom Cargo targets/profiles. It copies
both runtime resource sets: syntax collections and their download catalog, plus
the LSP server catalog. It does not install language servers or download grammars.
Without the syntax catalog a new language can appear as TXT; without the LSP
catalog no catalogued server is registered. A temporary syntax download status
then disappears when complete, leaving no server status to replace it. Restart
the application after staging or updating catalogs; both are read at startup.

## System dependencies (Debian / Ubuntu)

```sh
sudo apt install build-essential \
    libfreetype-dev libfontconfig-dev \
    libegl1-mesa-dev libgl1-mesa-dev libgles2-mesa-dev \
    libwayland-dev libxkbcommon-dev fonts-noto-cjk fonts-noto-color-emoji
```

A running X11 or Wayland desktop session is required to open the window.
(On WSL2, enable WSLg and install the packages above inside the distro.)

## Project layout

```
loom/
├── assets/          # UI resources and language metadata/query templates
├── crates/
│   ├── editor-core/       # GUI-free editing core
│   └── workspace-tests/   # GUI-free workspace domain tests
├── icons/           # application icons for every platform
├── scripts/         # packaging, build-cache and architecture checks
├── src/             # application, editor, terminal, Git and UI modules
└── vendor/vt100/    # terminal parser with Loom's patches
```

## Tests

The framework-independent editing core lives in
[`crates/editor-core`](../crates/editor-core/README.md). Run its tests without the
GUI with `cargo test --locked -p loom-editor-core`; run all application and core
regressions with `node scripts/build/cargo.mjs test --locked --workspace`. Workspace domain rules also
have a GUI-free verification entry point: `cargo test --locked -p loom-workspace-tests`.

`node scripts/architecture/check.mjs` enforces the module boundaries that CI
checks.

## Continuous integration and releases

GitHub Actions checks formatting, Clippy and architecture boundaries on Linux,
then tests and builds Loom on Linux x64, Windows x64 and macOS (universal).
All host CI and release jobs call the shared dependency preparer before host tests,
linting or compilation, then expose build artifacts. Managed inputs are cached by
OS, architecture and dependency manifest/implementation/lock hashes. macOS packaging
prepares each target separately before merging the binaries. Core-only CI remains
independent of CMake and Skia.

Each platform has its own release workflow (`release-windows.yml`,
`release-macos.yml`, `release-linux.yml`). Each validates the requested
version against `Cargo.toml` and builds packages with the scripts in
[`scripts/package`](../scripts/package/README.md). Application tag pushes also
run **Prepare release draft**, which collects English `Release Notes:` entries
from commit bodies and creates a GitHub Release draft. Platform workflows
upload their packages to that draft without changing its notes. Review the
notes, all build results and packages before publishing the draft on GitHub.

No per-version release note files are maintained. Record Added, Improved,
Changed, Removed, Fixed or Upgrade entries in the relevant commit body, or
`- N/A` for internal changes. Keep the section when squash-merging a PR. Before
releasing, sync
the root package version and lock entry, the CI version, and the default
version inputs in all four release workflows. The scripts require an existing
tag matching the build commit and never create or move tags. See
[Release notes and drafts](../scripts/release/README.md) for the format,
local preview command and manual workflow behavior.

Build the packages for the current platform locally with:

```sh
node scripts/package/package.mjs all
```
