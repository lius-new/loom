# Installed language resources

This directory contains the complete Markdown and JSON packages shipped with
Loom: standard metadata, WASM modules, queries, licenses and provenance.
Application packaging copies these packages and language-packs.toml directly
from this checkout. It does not build or download language packages.

language-packs.toml is the released package catalog; producer recipes live in
scripts/language-packs/build.toml. Markdown and JSON remain in that same batch
producer. Their validated WASM files were manually placed here and committed.
Running the producer does not rewrite the installed packages in this directory.

The catalog stays empty until publication succeeds. Automatic application
resource downloads are a separate integration.

See [producer workflow](../../scripts/language-packs/README.md) and
[runtime format](../../docs/languages.md).
