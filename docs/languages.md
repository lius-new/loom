# Language packages

Every Tree-sitter grammar is a dynamic language package, including JSON and
Markdown (block and inline). Loom links the generic Tree-sitter engine and its Wasmtime WASM runtime;
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

## Install the upstream Rust WASM download

Use the current WASM-enabled Loom build. Copy the metadata/query template from
assets/language-packs/rust/ to either installation location above, then download
[tree-sitter-rust.wasm v0.24.2](https://github.com/tree-sitter/tree-sitter-rust/releases/download/v0.24.2/tree-sitter-rust.wasm)
into its modules/ directory, keeping the original filename:

~~~text
language-packs/rust/
  package.json
  modules/tree-sitter-rust.wasm
  grammars/rust/highlights.scm
  grammars/rust/SOURCE.md
~~~

The bundled Rust template already declares module: "rust". It discovers the
standard download name automatically; no manifest edit, DLL, local compiler or
editor rebuild is needed. Restart Loom and open a .rs file. This provides file
recognition and syntax highlighting; rust-analyzer is separate for LSP features.
The same WASM bytes work across supported host operating systems and CPUs.

The verified upstream input has SHA-256:
24c89bd9252255e4aebbcbd7d2d308bd92c86dd95a130fdc80efa49577b8d738.
The application does not download modules itself.

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
      tree-sitter-python.wasm   # unchanged upstream WASM download
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
    "python": { "module": "tree-sitter-python.wasm" }
  }
}
~~~

Native libraries export the standard C entry point tree_sitter_python(), returning
const TSLanguage*. Standard Tree-sitter WASM modules expose the corresponding
entry point through Tree-sitter's official WasmStore API. symbol is optional and defaults to
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

Explicit module filenames select exactly one file: tree-sitter-python.wasm on
any platform, python.dll on Windows, python.so or libtree-sitter-python.so on
Linux, and python.dylib or python.so on macOS. Names permit lowercase letters,
digits, underscores and hyphens followed by .wasm/.dll/.so/.dylib; paths and extra
suffixes are rejected.

For a stem such as module: "rust", discovery checks only the package's modules/
directory, in this order: tree-sitter-rust.wasm, rust.wasm, then rust.dll (Windows),
rust.so (Linux) or rust.dylib (macOS). It never adds a lib prefix. WASM takes
precedence when both formats are supplied. If the chosen WASM is corrupt, it
fails for that language; it does not silently run a different native module.
An explicit native filename continues to select native execution.

WASM must be a standard Tree-sitter language module, with compatible Tree-sitter
ABI and dynamic-link metadata, such as the upstream release asset. Arbitrary
WASI programs, component-model modules, source files and native Node extensions
are not language modules. No WASI SDK, Tree-sitter CLI or C/Rust compiler is
needed on an end user's computer. Native modules remain supported and must match
the OS and CPU. A provider that only has grammar source must still produce a
standard module separately; Loom never compiles language source.

Query resources use the grammar key as their directory name. highlights.scm
supplies highlight captures, SOURCE.md records provenance and licenses, and
injections.scm is optional. Queries must match the parser version and Loom's
supported captures/predicates; Neovim-specific query extensions are not
implicitly supported just because its native parser can be loaded.

## Lazy loading and lifecycle

Startup reads only package manifests, validates descriptors and freezes registry
ids for the process. File recognition, directory browsing, search and hidden tab
restoration do not open queries or native modules. The syntax worker loads the
required grammar and queries when content is displayed. The first WASM request
initializes one shared Wasmtime engine and compiles that module in the background;
native-only use does not initialize it. Every document/layer parser owns a
separate WASM store; immutable languages and queries are shared safely. Off-screen Markdown
fences do not load their languages. Markdown and JSON follow exactly the same
path as all other packages. Repeated requests reuse resources and failure results.

A missing/corrupt module, invalid export, incompatible Tree-sitter ABI, execution
store setup failure or bad/missing query falls back to plain text for that language. Other packages keep working.
Distinct failures are logged once. Installing, disabling or changing metadata
requires a restart; there is no rescan, hot reload or module unloading. Native
library handles remain alive until process exit because existing trees and
queries hold pointers into their modules. WASM languages are reference-counted
and remain usable after their loading store is dropped.

Native packages execute trusted code inside the editor process. Only install
packages you trust. macOS modules must satisfy the OS code signing requirements;
WASM files are copied unchanged and are not signed as Mach-O libraries; the app
includes the allow-jit entitlement for Wasmtime. Native third-party libraries use Apple's
[Disable Library Validation entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.disable-library-validation).

Limits: 128 package directories per discovery root and 128 active packages,
1 MiB per manifest, 128 grammars, 128 package language descriptions and 64 servers,
64 loaded native modules, 64 cached WASM languages, 32 MiB per WASM file, 4 MiB per query and 8 MiB cached query text. Installing a
package does not launch an LSP server; document/server settings govern lifecycle.

## Build, verification and release inputs

The repository has no language producer workspace and no loom-grammar-* wrapper.
The host enables Tree-sitter's official wasm feature (Wasmtime); its build needs
CMake on PATH. The host workspace and Cargo.lock contain no concrete grammar crate, including
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
need universal native modules; WASM resources are platform-independent. The
packager signs native modules before the app and preserves WASM bytes.

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
complete eight-package native regression collection, plus the pinned WASM input:

~~~sh
node tests/language-runtime/run.mjs --packages /path/to/prebuilt-packages --wasm-rust /path/to/tree-sitter-rust.wasm --offline
~~~

A WASM-only regression needs no native language collection:

~~~sh
node tests/language-runtime/run-wasm.mjs
# With a local upstream download and cached Rust dependencies:
node tests/language-runtime/run-wasm.mjs --wasm-rust /path/to/tree-sitter-rust.wasm --offline
~~~

Without --wasm-rust, the test runner downloads and verifies a pinned upstream
release into .packaging/downloads/ (or uses it offline if cached). CI runs this
suite on every host platform even when no native artifact feed is configured.
This is test tooling, not an application downloader. Regression cases cover
cold zero-load discovery, independent stores, external scanners, incremental
edits, visible injections, corrupt modules, bad exports and restart/disable.

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
