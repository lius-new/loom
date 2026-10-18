# Built-in grammar resources

One directory per grammar, named in lowercase (`rust`, `c_sharp`, `tsx`), each
holding exactly:

- `highlights.scm` — the highlight query.
- `SOURCE.md` — upstream repository and commit, license, whether the query was
  modified, and the grammar crate version it matches.

`build.rs` embeds these files and fails on anything else. The directories are
added together with the Tree-sitter integration; until then no language
description names a grammar.
