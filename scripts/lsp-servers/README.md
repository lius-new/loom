# Language server production

These tools produce the packs Loom installs its language servers from, and the
catalog that describes them. Loom itself ships no server and no server definition:
everything below is run by the `Language servers` workflow, not on a developer's
machine.

## Files

- `build.toml`: the recipes, and the only place servers are defined. A **pack** is
  an installable archive set (an upstream release, an npm package with its
  dependencies, or a Go module); a **server** is a way of starting what a pack
  contains: its command, the languages it serves and how it finds a project root.
  Several servers can share one pack. A command always starts with `${pack}/` or
  `${runtime.<id>}/`; it never names a program found on `PATH`.
- `pins.toml`: the SHA-256 and size of every upstream archive and license file,
  written by `pin.mjs`. Nothing is downloaded that is not pinned here.
- `npm/<pack>/`: `package.json` and `package-lock.json` for each npm pack. The
  lockfile pins every dependency by integrity hash; installs run with
  `npm ci --ignore-scripts`.
- `assets/lsp-servers/lsp-servers.toml`: the published catalog. It is generated from
  the recipes and the release by the workflow, never edited by hand, and lists no
  server until one has been published. The application reads it; see
  [runtime behavior](../../docs/settings.md#language-servers).

## Pack kinds

| Kind | Source | Built |
| --- | --- | --- |
| `archive` | an upstream release asset per platform (zip, tar.gz, tar.xz, gz or a raw executable) | repackaged unchanged |
| `npm` | exact package versions and a lockfile | once, for every platform (`any`) |
| `go` | a Go module at a fixed version, with a fixed Go toolchain | cross-compiled for each platform |

Platforms are `windows`, `macos` and `linux` on `x86_64` and `aarch64`; a pack that
has no archive for a platform is simply unavailable there. An archive is built to
`<pack>-<platform>.tar.gz` with one top-level `<pack>/` directory holding the files,
`licenses/` and `PROVENANCE.md`. On Windows the loader appends `.exe` to a command
that names an executable without an extension.

A pack may `require` another by major version (`node@22`). Node is itself a pack, and
the npm servers run on it through `${runtime.node}`.

## Changing a version

Edit the version or tag in `build.toml`, then refresh the pins and review them:

```sh
node scripts/lsp-servers/pin.mjs --packs rust-analyzer
node scripts/lsp-servers/check.mjs
```

`pin.mjs` asks GitHub and nodejs.org for the published checksum and hashes the
download only when none is published; for npm packs it rewrites the lockfile. Commit
`build.toml`, `pins.toml` and `npm/`. The offline checks and unit tests need no
network and build nothing:

```sh
node scripts/lsp-servers/check.mjs
node --test scripts/lsp-servers/pipeline.test.mjs scripts/lsp-servers/catalog-sync.test.mjs
```

Both import the helpers of the language package pipeline, so install its tools first
(`npm ci --ignore-scripts --prefix scripts/language-packs`).

## Build, validate, publish

The manually dispatched `.github/workflows/lsp-servers.yml` runs on GitHub:

1. **build** (Linux): checks the recipes, pins and lockfiles, builds the selected
   packs for every platform into `.packaging/lsp-servers/dist`, with
   `build-report.json` and `SHA256SUMS`.
2. **validate** (Linux, Windows, macOS): unpacks the packs built for that runner's
   platform, starts every server and completes an LSP `initialize`/`shutdown`
   exchange with it (`validate.mjs`).
3. **publish** (only when selected, from a branch): `publish.mjs prepare` generates a
   candidate catalog from the recipes and the build report, `verify` checks every
   archive, then the release is created as a draft, uploaded, verified on the server,
   published, and only then promoted to `assets/lsp-servers/lsp-servers.toml`, which
   the workflow commits back to the branch.

Resource tags start with `lsp-servers-` and are never marked latest. Existing assets
are never overwritten; a pack that changed needs a new tag. Publish only from the
workflow: a build on Windows does not preserve executable permissions.

The catalog gets an entry only when its pack was built for every platform it
declares, so a partial run (`--platforms`) can never be prepared for publication.
