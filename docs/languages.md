# Language packages

Every Tree-sitter grammar is a dynamic language package, including JSON and
Markdown (block and inline). Loom itself links only the Tree-sitter engine;
plain text works without any installed package. Bundled packages are enabled by
default, which does not load their native libraries or queries at startup.

## Discovery and installation

At startup Loom scans the immediate child directories in two locations:

| Source | Location |
| --- | --- |
| Bundled, Windows/Linux | language-packs/ beside the executable |
| Bundled, macOS | Loom.app/Contents/Resources/language-packs/ |
| User, Windows | %LOCALAPPDATA%/Loom/language-packs/ |
| User, macOS | ~/Library/Application Support/Loom/language-packs/ |
| User, Linux | $XDG_DATA_HOME/loom/language-packs/, otherwise ~/.local/share/loom/language-packs/ |

An XDG data location must be absolute. Resource locations do not depend on the
working directory. Missing directories are valid; the application does not create
them during discovery. Install a trusted package by extracting its complete
folder into the user location, then restart Loom. No central catalog, source
edit or recompilation of the editor is required. There is no downloader or
package management UI in this first version.

Each directory with package.json is one self-contained package. Other files,
unrelated directories and directory symlinks are not package candidates. A valid
user package with the same id replaces the entire bundled package, including
its queries and native modules. User packages never borrow a missing module
from a bundled package or Cargo build. An invalid replacement leaves the bundled
package available. Setting enabled to false removes that id from the discovered
set; it can disable a bundled package using a user copy of its manifest.

Packages are processed in sorted directory order, bundled first, then user.
Conflicting grammar names, language names/aliases, server names or file rules
across different ids reject the incoming package and record an error. One bad
package does not invalidate the rest. Plaintext is reserved by the editor.

## Independent package format

Example installed layout (Windows):

~~~text
language-packs/
  fixture/
    package.json
    grammars/
      fixturegrammar/
        highlights.scm
        SOURCE.md
        injections.scm          # optional
    modules/
      loom_grammar_rust.dll
~~~

A minimal package can reuse a separately compiled Rust grammar to demonstrate
a new language and extension entirely through data:

~~~json
{
  "schema": 1,
  "id": "fixture",
  "enabled": true,
  "languages": [{
    "name": "fixturelang",
    "display_name": "Fixture",
    "badge": "FIX",
    "variants": [{ "id": "fixturelang", "grammar": "fixturegrammar" }],
    "files": [{ "extensions": ["fixture"], "variant": "fixturelang" }]
  }],
  "servers": [],
  "grammars": {
    "fixturegrammar": {
      "module": "loom_grammar_rust",
      "symbol": "loom_tree_sitter_rust"
    }
  }
}
~~~

The directory name must equal id. Package ids, grammar names, module basenames
and exported symbols use lowercase ASCII letters, digits and underscores,
starting with a letter. Unknown manifest fields and unsupported schema versions
are rejected. enabled defaults to true; servers defaults to an empty list.
Language and server descriptions use the existing descriptor format. Grammar
and server references must resolve inside the same package. A package can supply
multiple grammars and several entry points from one native module, as Markdown
and TypeScript do. Native entry points have the C signature returning a pointer
to a Tree-sitter TSLanguage; generated tree_sitter_* functions and Loom wrapper
exports both work. No application-specific Rust ABI is required.

Ship platform and CPU compatible binaries. Module names gain .dll on Windows,
lib prefix and .so on Linux, and lib prefix and .dylib on macOS. For example,
loom_grammar_rust becomes libloom_grammar_rust.so on Linux. Query resources use
the grammar key as their directory name. highlights.scm defines captures and
SOURCE.md records grammar/query provenance and licenses. injections.scm is
optional. A downloaded source grammar or query file alone is not a complete
native package.

## Lazy loading and lifecycle

Startup reads only package manifests, validates descriptors and freezes registry
ids for the process. File recognition, directory browsing, search and hidden tab
restoration do not open queries or native modules. The syntax worker loads the
required grammar and queries when content is displayed. Off-screen Markdown
fences do not load their languages. Markdown and JSON follow exactly the same
path as all other packages. Repeated requests reuse resources and failure results.

A missing module, invalid export, incompatible Tree-sitter ABI or bad/missing
query falls back to plain text for that language. Other packages keep working.
Distinct failures are logged once. Installing, disabling or changing metadata
requires a restart; there is no rescan, hot reload or module unloading. Native
library handles remain alive until process exit because existing trees and
queries hold pointers into their modules.

Native packages execute trusted code inside the editor process. Only install
packages you trust. macOS modules must satisfy the OS code signing requirements;
the app's signing configuration permits third-party libraries using Apple's
[Disable Library Validation entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.disable-library-validation).

Limits: 128 package directories per discovery root and 128 active packages,
1 MiB per manifest, 128 grammars, 128 package language descriptions and 64 servers,
64 loaded native modules, 4 MiB per query and 8 MiB cached query text. Installing a
package does not launch an LSP server; document/server settings govern lifecycle.

## Development and release staging

Run cargo build --locked (or add --release) to build the editor and all eight
separate native modules, then cargo run --locked in the same profile. Building
only -p loom builds the editor without grammar modules. Tests build those modules
as development dependencies, which never enter the editor's normal dependency
graph. Syntax tests use isolated bundled packages, not the developer's user packs.

Bundled manifests live at assets/language-packs/<id>/package.json. build.rs copies
each manifest and its referenced queries from assets/grammars/ into
target/<profile>/language-packs/<id>/. No grammar query is embedded in the editor;
only the plain-text fallback descriptor is embedded in production. Development
builds may find bundled native modules in the Cargo profile/deps directory.
Release scripts copy modules into each package's modules/ directory. macOS
packaging combines architecture slices and signs native modules before the app.
Linux/macOS runtime, signing and installer verification still need those platforms.
