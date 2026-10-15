# Updates

Packaged builds of Loom check for a new release about ten seconds after
startup and then once a day. When one is available, a button appears in the
title bar, left of Settings:

| Install | Button | Clicking it |
| --- | --- | --- |
| Windows installer (`…-setup.exe`) | **Update to x.y.z** | Downloads and verifies the new installer, then shows **Restart to update** |
| Windows portable, macOS, Linux | **Loom x.y.z available** | Opens the release page to download it |

**Restart to update** closes Loom the usual way: unsaved files are offered
for saving first, and the session is restored afterwards. The installer then
runs silently, keeping the install folder and the options chosen at install
time, closes any other Loom windows and starts Loom again. Running terminal
programs end with the restart. If the restart is cancelled, the installer runs
the next time Loom quits.

Turn checks off with Settings → Updates → **Check for updates**, or
`"check_for_updates": false` in `settings.json`. Builds made with
`cargo build` never check.

Settings → About → **Version** shows the running version and the result of
the last check (up to date, checking, or which release is available).
**Check now** checks right away, even with automatic checks off; once a
release is found, the same link downloads, installs or opens it like the
title bar button.

## How it works

Each release carries a signed manifest per platform,
`update-windows-x86_64.json`, `update-macos-universal.json` and
`update-linux-x86_64.json`. Loom downloads it from
`https://github.com/lius-new/loom/releases/latest/download/` (the newest
release that is not a pre-release) and:

1. verifies its Ed25519 signature with the public key built into Loom
   (`src/infrastructure/update.rs`), so only manifests signed with the
   release key are trusted;
2. checks that it names this platform and a version newer than the running
   one;
3. downloads the installer and checks its size and SHA-256 against the
   manifest before it is ever run.

Requests honor `HTTPS_PROXY`/`HTTP_PROXY` and use HTTPS only. Check failures
are silent; the next check tries again. A failed download shows an error and
offers the update again.

## Releasing

The release workflows run `node scripts/package/package.mjs manifest` after
building the packages and publish `update-<platform>.json` with them. The step
needs the `LOOM_UPDATE_SIGNING_KEY` repository secret (the PKCS#8 PEM private
key) and fails without it. See
[Packaging](../scripts/package/README.md#update-manifests) for creating or
replacing the key.

## Testing against a local feed

`LOOM_UPDATE_FEED=<base url>` makes any build, including a debug build, read
its manifest from another place. A loopback feed may use plain HTTP and skips
the proxy:

```bash
LOOM_UPDATE_FEED=http://127.0.0.1:8765 cargo run
```

The manifest must still be signed with the release key, and its package URLs
must use HTTPS. A debug build counts as a Windows installer install only when
an `unins000.exe` file sits next to `Loom.exe`. To check that a published
feed verifies:

```bash
cargo test live_feed -- --ignored --nocapture
```
