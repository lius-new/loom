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
Loom.exe
language-packs/
  python/
    package.json
    grammars/
      python/
        highlights.scm
        SOURCE.md
        injections.scm          # optional
    modules/
      python.dll                # unchanged standard Tree-sitter library
~~~

A third-party language is registered entirely through files. For example:

~~~json
{
  "schema": 1,
  "id": "python",
  "languages": [{
    "name": "python",
    "display_name": "Python",
    "badge": "PY",
    "variants": [{ "id": "python", "grammar": "python" }],
    "files": [{ "extensions": ["py", "pyw"], "variant": "python" }]
  }],
  "grammars": {
    "python": { "module": "python.dll" }
  }
}
~~~

The DLL exports the standard C entry point tree_sitter_python(), returning
const TSLanguage*. The symbol field is optional and defaults to
tree_sitter_<grammar>. Specify it explicitly when the resource key differs from
the upstream grammar name, such as a custom variant reusing tree_sitter_python.
No Loom-specific export, Rust wrapper, Cargo dependency or editor rebuild is
needed. Loom contains one generic Tree-sitter engine and one generic loader.

The directory name must equal id. Package ids, grammar names and export symbols
use lowercase ASCII letters, digits and underscores, starting with a letter.
Unknown manifest fields and unsupported schema versions are rejected. enabled
defaults to true; servers defaults to an empty list. Language and server
references must resolve inside the same package. Several grammar entries may
share a native module, or use separate standard modules (Markdown block/inline,
TypeScript/TSX).

Use a module filename to preserve it exactly: python.dll on Windows, python.so
or libtree-sitter-python.so on Linux, and python.dylib or python.so on macOS.
Filenames permit lowercase letters, digits, underscores and hyphens followed by
.dll, .so or .dylib; paths and extra suffixes are rejected. Alternatively,
module: "python" adds .dll/.so/.dylib for Windows/Linux/macOS, with no lib prefix.
Supply binaries matching the OS, CPU and supported Tree-sitter ABI. Source files
such as grammar.js or parser.c cannot be loaded as a native module. A prebuilt
module is copied unchanged; building from source is the resource provider's job,
using upstream tools such as tree-sitter build, independently of Loom.

Query resources use the grammar key as their directory name. highlights.scm
supplies highlight captures, SOURCE.md records provenance and licenses, and
injections.scm is optional. Queries must match the parser version and Loom's
supported captures/predicates; Neovim-specific query extensions are not
implicitly supported just because its native parser can be loaded.

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

## Build, verification and release inputs

The repository has no language producer workspace and no loom-grammar-* wrapper.
The host workspace and Cargo.lock contain no concrete grammar crate, including
development and build dependencies. Host build.rs does not read or stage
language files. Both commands work without any external language resources:

~~~sh
cargo build --locked
cargo test --locked --workspace
~~~

assets/language-packs/ holds only bundled language descriptions, queries and
provenance. It contains no grammar source, native module or build project. These
files are resource templates; they are not automatically installed by cargo
build. A source build runs as plain text until complete compatible resources are
placed beside the executable or in the user directory. JSON and Markdown are
default-enabled resources in official distributions and remain demand-loaded.

Application packaging consumes complete prebuilt resources via LOOM_LANGUAGE_PACKS:

~~~sh
# Set LOOM_LANGUAGE_PACKS to an existing collection for the target platform.
node scripts/package/stage-languages.mjs /path/to/app/language-packs
~~~

It copies manifests, queries and native modules, without compiling any grammar
or consulting Cargo profile/deps directories. The source collection can come
from any provider following the file contract. macOS universal distributions
need universal modules; the packager signs supplied modules before the app.

CI and release workflows can fetch a pinned tar.gz containing package folders
at its root. Configure repository variables LOOM_LANGUAGE_PACKS_<TARGET>_URL and
LOOM_LANGUAGE_PACKS_<TARGET>_SHA256, where TARGET is WINDOWS_X86_64,
LINUX_X86_64 or MACOS_UNIVERSAL. This is a build-time resource feed, not an
application downloader. The fetch step verifies the checksum and requires
enabled JSON and Markdown. Releases require this input; CI without a feed still
builds/tests and uploads the host, explicitly without language resources.
No external provider URL is hardcoded or created by this change.

Real parser, injection, worker and installed-resource regressions live in the
independent tests/language-runtime workspace. It depends only on the generic
engine and production sources; it never builds language modules. Supply the
complete eight-package regression collection, with standard native modules:

~~~sh
node tests/language-runtime/run.mjs --packages /path/to/prebuilt-packages --offline
~~~

Use --new-package <external-folder> --sample <new-file> to additionally test a
genuinely new grammar (for example Python) absent from the baseline collection.

The production-mode probe copies one compiled executable to a temporary app
folder. It verifies plain text with no resources, installation beside that
executable, moving the same resources to the isolated user directory, new
extension recognition and real highlighting, lazy module loads, disabling on
restart, and an unchanged executable hash. Host unit tests need no native
resources; manual UI syntax benchmarks use LOOM_TEST_LANGUAGE_PACKS explicitly.

~~~sh
node scripts/architecture/standalone-host.mjs --offline
~~~

Omit --offline on a fresh dependency cache (as CI does).

This physical-removal check excludes all language resources and the external
runtime verifier, then runs the host formatter, dependency check, full build
and workspace tests. The production host has no source reference, feature or
custom cfg switch for the external verifier.
