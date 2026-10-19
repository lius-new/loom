# Building Loom

## Toolchain

Rust 1.97.1, Clippy and rustfmt are pinned in
[`rust-toolchain.toml`](../rust-toolchain.toml); CI uses the same compiler.

```sh
cargo build --release --locked
cargo run --release --locked
```

`lgui` uses Skia for rendering and Winit for the window. The renderer is the
**software** Skia backend (`renderer-skia-software`), which presents via
`softbuffer`. `skia-safe` downloads a prebuilt Skia archive automatically at
build time. Windows builds with an existing Skia archive can use the
[verified local cache helper](../scripts/build/README.md), including for
offline builds and baseline checks.

The project follows the `main` branch of the lgui repository. `Cargo.lock`
pins the exact lgui commit used by a build. Run `cargo update -p lgui` when you
want to update that pinned commit.

The default build also builds all separate native language modules, including
Markdown and JSON. Plain text needs no package; all parsers are shared libraries
loaded only when visible content needs them. The build stages independent
package manifests and query files in `target/<profile>/language-packs/`. Build the
workspace once before using cargo run; building only -p loom does not build
the separate modules. See [Language packages](languages.md) for the disk format.

## System dependencies (Debian / Ubuntu)

```sh
sudo apt install build-essential \
    libfreetype-dev libfontconfig-dev \
    libegl1-mesa-dev libgl1-mesa-dev libgles2-mesa-dev \
    libwayland-dev libxkbcommon-dev
```

A running X11 or Wayland desktop session is required to open the window.
(On WSL2, enable WSLg and install the packages above inside the distro.)

## Project layout

```
loom/
├── assets/          # UI icons, file-type icons and the default keymap
├── crates/
│   ├── editor-core/       # GUI-free editing core
│   ├── workspace-tests/   # GUI-free workspace domain tests
│   └── language-packs/    # separately built native grammar modules
├── icons/           # application icons for every platform
├── scripts/         # packaging, build-cache and architecture checks
├── src/             # application, editor, terminal, Git and UI modules
└── vendor/vt100/    # terminal parser with Loom's patches
```

## Tests

The framework-independent editing core lives in
[`crates/editor-core`](../crates/editor-core/README.md). Run its tests without the
GUI with `cargo test --locked -p loom-editor-core`; run all application and core
regressions with `cargo test --locked --workspace`. Workspace domain rules also
have a GUI-free verification entry point: `cargo test --locked -p loom-workspace-tests`.

`node scripts/architecture/check.mjs` enforces the module boundaries that CI
checks.

## Continuous integration and releases

GitHub Actions checks formatting, Clippy and architecture boundaries on Linux,
then tests and builds Loom on Linux x64, Windows x64 and macOS (universal).
Successful runs expose packaged builds as workflow artifacts.

Each platform has its own release workflow (`release-windows.yml`,
`release-macos.yml`, `release-linux.yml`). Each validates that the requested
version matches `Cargo.toml`, builds the packages with the scripts in
[`scripts/package`](../scripts/package/README.md), and publishes them to the
GitHub release. Push a version tag such as `v0.1.0`, or run a workflow
manually with `0.1.0`.

Build the packages for the current platform locally with:

```sh
node scripts/package/package.mjs all
```
