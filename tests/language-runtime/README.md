# Independent language runtime verification

This Cargo workspace compiles production domain, syntax, loader and worker
sources directly. It has no concrete grammar crate, Rust wrapper or language
build project dependency. Use standard upstream native modules.
Supply a complete prebuilt collection (the regression fixtures expect the eight
bundled packages), using either standard or legacy manifests. Standard input
directory names may retain the upstream names. The main tests exercise the
production loader against that input. legacy-fixture.mjs makes a separate test-only
view for old-manifest mutation/migration cases; it copies prebuilt modules and
queries, never builds grammar code, and is not shipped to users:

~~~sh
node run.mjs --packages /path/to/prebuilt-packages --wasm-rust /path/to/tree-sitter-rust.wasm
~~~

Use --offline for cached Rust dependencies, or --probe-only to run just the
production-mode install/restart probe. The probe verifies plain text without
resources, JSON/Markdown/custom-extension highlighting after portable and user installation,
an unchanged executable hash,
no implicit Rust activation, and disabling on restart. It runs in a temporary
location with isolated user data variables and removes its files after exit.

The host's default cargo test does not enable these parser tests and requires no
language artifacts. Host metadata unit fixtures contain no parser/query payload.

To additionally verify a genuinely new grammar outside the baseline collection,
supply any complete external package and a sample whose extension it registers:

~~~sh
node run.mjs --packages /path/to/baseline --new-package /path/to/python --sample /path/to/new.py --probe-only
~~~

The same production-mode executable must show the sample as plain text before
installation, then recognize and highlight it after copying only those resource
files into the user directory. No host source, dependency or binary changes.

The WASM-only suite has no native grammar input:

~~~sh
node run-wasm.mjs
node run-wasm.mjs --wasm-rust /path/to/tree-sitter-rust.wasm --offline
~~~

Both runners use the unchanged Rust v0.24.2 upstream WASM asset, verified by its
pinned SHA-256 in wasm-fixture.mjs. Online runs cache it under .packaging/downloads/;
offline runs require that cache or an explicit --wasm-rust path. CMake must be on
PATH when compiling the generic runtime. No grammar source or language compiler
is invoked. WASM tests include cold engine initialization, independent parser
stores, incremental edits, scanner state, injections, invalid modules/exports and
store failure fallback. The production probe installs only a .wasm file and
upstream tree-sitter.json and queries/ files, with no package.json or SOURCE.md.
It verifies both resource locations, a metadata-declared new extension and removal
on restart while the executable hash remains unchanged. The native baseline
continues to verify migration compatibility with existing legacy collections.
