# Shared build entry point

`rust-toolchain.toml` and `Cargo.lock` pin Rust and crate inputs. Use Node.js 20+
and the platform C/C++ SDK; CI selects Node 24. The shared preparer downloads and
verifies private CMake and target-specific Skia before the first host build.

```sh
node scripts/build/cargo.mjs build --release --locked
node scripts/build/cargo.mjs test --locked --workspace
node scripts/build/cargo.mjs run --release --locked
node scripts/dependencies/ensure.mjs
node scripts/build/cargo.mjs build --release --locked --offline
```

`--offline` / `--frozen` require populated dependency and Cargo caches and never
silently switch to network downloads. Formatting, Cargo informational commands,
and tests selecting only loom-editor-core / loom-workspace-tests skip preparation.
`--target` (including `--target=...`) and CARGO_BUILD_TARGET select Skia inputs.

PowerShell remains a thin compatibility entry point:

```powershell
./scripts/build/cargo.ps1 -CargoArguments @('test', '--locked', '--offline', '--workspace')
./scripts/build/cargo.ps1 -SkiaArchive 'D:/cache/skia.tar.gz' -CargoArguments @('build', '--release', '--locked')
./scripts/baseline/run.ps1 -Offline -Samples 5
```

An explicit Skia archive must match the pinned target, features, SHA-256 and embedded
version/key. CMake is still prepared automatically. The old Windows-only cache
manifest/resolver has been replaced by [scripts/dependencies](../dependencies/README.md).
Local wrappers scope environment changes to children; the baseline script restores
its PATH and Skia environment even on failure. The wrapper forwards Cargo's exit
code. Bare Cargo does not bootstrap external dependencies.
