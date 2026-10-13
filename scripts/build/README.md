# Reproducible local builds

`rust-toolchain.toml` pins Rust 1.97.1 with Clippy and rustfmt. CI and the
Windows release workflow select the same compiler. `Cargo.lock` pins the
dependencies, including lgui; use `--locked` when building or testing.

```powershell
cargo build --release --locked
cargo test --locked --workspace
```

## Windows Skia cache

Cargo's `--offline` controls Cargo dependency resolution. The skia-bindings
build script can still attempt its own download. When that fails, its source
build requires a separate LLVM installation; Rust's bundled LLVM does not
supply that installation.

The project can use the matching prebuilt archive through the upstream build
script's `SKIA_BINARIES_URL=file://...` support, without patching dependencies.

```powershell
./scripts/build/cargo.ps1 test --locked --offline --workspace
./scripts/build/cargo.ps1 build --release --locked --offline
./scripts/baseline/run.ps1 -Offline -Samples 5
```

These scripts find the archive under
`target/dependency-cache/skia/0.99.0/`, then in existing debug/release
`skia-bindings-*/out/.cache/` directories. The archive is not committed.
The manifest in `skia-cache.json` records the upstream URL, exact Windows x64
feature key and SHA-256. Its digest was obtained from the existing local archive;
the archive's embedded version/key were checked and the current build was
verified with it. This is a content pin, not a signature or independent upstream
attestation.

To prepare another machine, download the manifest's `source` URL into the
dependency-cache directory or provide an existing archive explicitly:

```powershell
./scripts/build/cargo.ps1 -SkiaArchive 'D:/cache/skia.tar.gz' -CargoArguments @('test', '--locked', '--offline')
./scripts/baseline/run.ps1 -SkiaArchive 'D:/cache/skia.tar.gz' -Offline
```

An explicit missing archive, wrong SHA-256 or changed locked Skia version
fails before compilation. If no local archive exists, the normal upstream
build path remains available. An existing `SKIA_BINARIES_URL` is respected
unless `-SkiaArchive` is explicitly supplied. Environment changes are limited
to the script and its children and restored when it exits.

The Windows archive does not support Linux, macOS, ARM64 or a different Skia
feature set. Review the manifest when changing the renderer or Skia dependency.

Run cache guard checks with:

```powershell
./scripts/build/skia-cache.test.ps1
```
