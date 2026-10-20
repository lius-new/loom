# Installed language resources

This directory contains the complete Markdown and JSON packages shipped with
Loom: standard metadata, WASM modules, queries, licenses and provenance.
Application packaging copies these packages and language-packs.toml directly
from this checkout. It does not build or download language packages.

language-packs.toml is the released package catalog; producer recipes live in
scripts/language-packs/build.toml. Markdown and JSON remain in that same batch
producer. Their validated WASM files were manually placed here and committed.
Running the producer does not rewrite the installed packages in this directory.

The catalog contains successfully published language packages. At runtime Loom
reads this shipped snapshot once and automatically installs missing packages when
visible content needs them. Downloads are verified, cached in the user directory
and activated without restart. Bundled and manually installed collections remain
authoritative. The application does not fetch newer catalog snapshots.

See [producer workflow](../../scripts/language-packs/README.md) and
[runtime format](../../docs/languages.md).
