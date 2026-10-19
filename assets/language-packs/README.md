# Bundled language resource templates

These folders contain only language metadata, highlighter queries and
provenance. There is no language build project or Rust grammar wrapper here.
The generic loader reads the same format for bundled and user-added languages.

To install a language, obtain a standard prebuilt Tree-sitter module for your
platform and place it in that language's modules/ directory alongside the
package.json and grammars/ resources. The default module stem gains .dll, .so
or .dylib for Windows, Linux or macOS. An explicit filename is also accepted,
without renaming. Entry points default to tree_sitter_<grammar>.

Markdown block and inline use separate modules, as do TypeScript and TSX.
All provided manifests are enabled by default; discovery does not load modules
or queries. Restart after adding or changing resources.

These templates are not automatically copied by cargo build. The application
packager consumes complete externally supplied resources via LOOM_LANGUAGE_PACKS.
See ../../docs/languages.md for installation, format and verification.
