# Third-party notices

Official Loom packages may include a managed Git runtime. Git is licensed under
the GNU General Public License version 2. Runtime packages must retain Git's
license, source-offer information, upstream version, download URL, and the
generated `runtime/git/MANIFEST.json` file.

The packaging pipeline treats the runtime as a separate executable component;
Loom invokes it through its command-line interface and does not link Git into
the Loom binary.

## Zed GPUI keymap

The key-context predicate language and binding resolution in
`src/input/context.rs`, `src/input/keymap.rs` and `src/input/dispatcher.rs` are
adapted from the `gpui` crate of Zed (https://github.com/zed-industries/zed,
`crates/gpui/src/keymap*` and `crates/gpui/src/key_dispatch.rs`), Copyright
Zed Industries, Inc., licensed under the Apache License, Version 2.0. The
adapted code was rewritten for Loom's types; its behavior and test cases follow
the original.
