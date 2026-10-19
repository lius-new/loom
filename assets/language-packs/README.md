# Standard language resource templates

Each child directory contains tree-sitter.json and the query paths it declares.
No language source, build project or prebuilt module is stored here. These files
are resource templates; cargo build does not stage them automatically.

For Rust, download the upstream v0.24.2 tree-sitter-rust.wasm into rust/ beside
its tree-sitter.json. Copy that complete directory under language-packs/ beside
Loom or in the platform user directory, then restart. Keep the upstream queries/
layout. No Loom package.json or SOURCE.md is needed. Rust's configuration and
queries are unmodified upstream resources; LICENSE accompanies them.

Other directories retain existing supported query content in standard layouts.
PROVENANCE.md is optional distribution documentation, not part of discovery or
runtime loading. Application editing/LSP defaults are separate from these files.

See [installation and format](../../docs/languages.md) for download links,
platform paths, resource resolution, compatibility and regression commands.
