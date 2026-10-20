# WASI character classification support

`iswcntrl.c` retains the `iswcntrl` implementation from WASI libc's musl source:

https://github.com/WebAssembly/wasi-libc/blob/23472ab971db4cdbdc9bb818134d352d08e9338c/libc-top-half/musl/src/ctype/iswcntrl.c

Only the locale-independent function is linked. The unused locale wrapper and
internal weak-alias declaration are omitted; classification ranges are unchanged.
The corresponding musl COPYRIGHT is retained here and copied into the package.

Git Commit's lexer imports this function. Tree-sitter's miniature WASM libc
exports `iswspace` but not `iswcntrl`, so this small implementation is linked into
that package rather than expanding the host ABI or linking the entire WASI libc.
