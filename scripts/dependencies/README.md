# Build dependencies

Local builds and GitHub Actions share this directory. Node.js 20+ and Rust plus
the platform C/C++ SDK are prerequisites. No npm packages, administrator installs,
system CMake or manual Skia/LLVM setup are needed by the managed build path.

```sh
node scripts/dependencies/ensure.mjs
node scripts/build/cargo.mjs build --release --locked
node scripts/package/package.mjs all
```

`manifest.json` pins CMake 3.31.6 and Skia 0.99.0, official archive URLs, SHA-256,
host layouts and target feature sets. CMake hashes come from Kitware's
[release checksum file](https://github.com/Kitware/CMake/releases/download/v3.31.6/cmake-3.31.6-SHA-256.txt);
Skia hashes come from the `digest` fields of the official
[release metadata](https://api.github.com/repos/rust-skia/skia-binaries/releases/tags/0.99.0).
Update version, URL, hash and feature set together when changing dependencies.
The preparer compares Skia against Cargo.lock and `cargo tree` for the requested
target, then checks the archive's embedded key/version. Unknown targets or feature
sets fail before compilation. Windows x64, Linux x64 and both macOS architectures
are pinned; other Skia targets require reviewed manifest entries first.

The private cache lives in `.packaging/dependencies`:

```text
archives/<sha256>/<official filename>
tools/<sha256>/content/<CMake distribution>
tools/<sha256>/receipt.json
```

All pinned network inputs (CMake/Skia, language/LSP resources and packaging
tools) share scripts/lib/download.mjs. Transient connection failures, HTTP 408,
429 and 5xx retry at most four attempts within a five-minute total download
budget, using exponential backoff and honoring Retry-After. Every attempt uses
its own temporary file; only a complete SHA-256-verified file is published.
Permanent HTTP failures and digest mismatches fail immediately.

Delay/failure injection regressions run without external network access:

~~~sh
node --test scripts/lib/download.test.mjs scripts/dependencies/dependencies.test.mjs
~~~

Archives are hashed on every reuse. Temporary downloads are verified before
publication. Extracted CMake files/symlinks are checked against a receipt and
repaired from the archive if missing or changed; concurrent extraction is locked.
Nothing changes the parent environment or installs tools globally. CMake's bin
directory is prepended to the Cargo child PATH and CMAKE points to that executable,
taking precedence over system tools. Skia uses rust-skia's `SKIA_BINARIES_URL=file://<native absolute path>`
support. Existing Skia URL/source-build switches are replaced for managed builds;
the upstream FORCE download flag must remain unset because it expects a Git
source checkout. An explicitly provided archive is checked with the same pins.

```sh
node scripts/dependencies/ensure.mjs --target x86_64-apple-darwin
node scripts/dependencies/ensure.mjs --no-skia
node scripts/dependencies/ensure.mjs --offline
node scripts/build/cargo.mjs build --release --locked --offline
node --test scripts/dependencies/dependencies.test.mjs
```

`--no-skia` prepares only CMake for independent Tree-sitter runtime tests.
`--offline` fails with a preparation command if an archive is missing/corrupt;
Cargo crates and the Rust toolchain must already be cached separately.
`--cache <path>` selects a private cache for isolated preparation.
`--skia-archive <path>` accepts an existing archive with the exact pinned bytes.
The Cargo wrapper also accepts that option. Formatting/core-only tests skip both
dependencies. macOS universal packaging prepares Skia separately for each target.

`.github/actions/dependencies` selects Node 24, caches the same private directory,
and calls `ensure.mjs --github` before any host test/lint/build in CI and release
workflows. That mode writes the verified bin path and Skia URL to GitHub's
environment files for subsequent steps. Cache keys include OS, architecture and
manifest/implementation/Cargo.lock hashes. All matrix platforms run the regression
tests and a real private-CMake/Actions-export smoke check. Inno Setup/MinGit retain their separate packaging roles; bundled language
resources are copied from assets/language-packs. This directory does not build
grammar resources.
