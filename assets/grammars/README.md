# Built-in grammar resources

One directory per grammar, named in lowercase (`rust`, `c_sharp`, `tsx`), each
holding exactly:

- `highlights.scm` — the highlight query.
- `SOURCE.md` — upstream repository and commit, license, whether the query was
  modified, and the grammar crate version it matches.

`build.rs` embeds these files and fails on anything else. A language description
names a grammar by directory name; the native parser behind the name is
registered in `src/features/syntax/grammars.rs`.
