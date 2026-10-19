# Third-party notices

## vt100 terminal parser

`vendor/vt100` contains vt100 0.16.2 by Jesse Luehrs, licensed under MIT
(see `vendor/vt100/LICENSE`). Loom modifies height resizing to preserve terminal
history and exposes the row displacement for selection tracking. Upstream:
https://github.com/doy/vt100-rust.

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

## Tree-sitter and language grammars

Syntax highlighting parses source with Tree-sitter and these grammars. All grammars, including JSON and Markdown,
ship as separate standard WASM or native modules loaded on demand, together with the highlight queries referenced by `assets/language-packs/<id>/tree-sitter.json`.
Optional `PROVENANCE.md` files name the upstream repository, commit,
license and any change (the queries of one JavaScript and TypeScript family are
only concatenated).

| Component | Version | Upstream | Copyright |
| --- | --- | --- | --- |
| tree-sitter | 0.27.1 | https://github.com/tree-sitter/tree-sitter | Max Brunsfeld |
| tree-sitter-rust | 0.24.2 | https://github.com/tree-sitter/tree-sitter-rust | Maxim Sokolov |
| tree-sitter-javascript | 0.25.0 (query for JavaScript), 0.23.1 (query for TypeScript and TSX) | https://github.com/tree-sitter/tree-sitter-javascript | Max Brunsfeld |
| tree-sitter-typescript | 0.23.2 | https://github.com/tree-sitter/tree-sitter-typescript | its contributors |
| tree-sitter-c-sharp | 0.23.5 | https://github.com/tree-sitter/tree-sitter-c-sharp | Max Brunsfeld, Damien Guard, Amaan Qureshi, and contributors |
| tree-sitter-json | 0.24.8 | https://github.com/tree-sitter/tree-sitter-json | Max Brunsfeld, Amaan Qureshi |
| tree-sitter-css | 0.25.0 | https://github.com/tree-sitter/tree-sitter-css | Max Brunsfeld, Amaan Qureshi |
| tree-sitter-html | 0.23.2 | https://github.com/tree-sitter/tree-sitter-html | Max Brunsfeld, Amaan Qureshi |
| tree-sitter-md | 0.5.3 (block and inline grammars) | https://github.com/tree-sitter-grammars/tree-sitter-markdown | MDeiml and contributors |

All of them are licensed under the MIT License:

> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in
> all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
> SOFTWARE.


## Wasmtime and Cranelift

Tree-sitter's WASM feature embeds Wasmtime and Cranelift 48.0.6/0.135.6 from
https://github.com/bytecodealliance/wasmtime. They are licensed under
Apache-2.0 WITH LLVM-exception. The Apache 2.0 text is included in LICENSE-APACHE;
the upstream LLVM exception follows:

--- LLVM Exceptions to the Apache 2.0 License ----

As an exception, if, as a result of your compiling your source code, portions
of this Software are embedded into an Object form of such source code, you
may redistribute such embedded portions in such Object form without complying
with the conditions of Sections 4(a), 4(b) and 4(d) of the License.

In addition, if you combine or link compiled forms of this Software with
software that is licensed under the GPLv2 ("Combined Software") and if a
court of competent jurisdiction determines that the patent provision (Section
3), the indemnity provision (Section 9) or other Section of the License
conflicts with the conditions of the GPLv2, you may retroactively and
prospectively choose to deem waived or otherwise exclude such Section(s) of
the License, but only in their entirety and only with respect to the Combined
Software.
