# Repository instructions

Every commit must contain exactly one `Release Notes:` section in its body.
Use English release-note bullets for user-facing changes. For internal changes,
include this explicitly:

```text
Release Notes:
- N/A
```

Do not automatically classify user-facing changes as N/A. Follow
[scripts/release/README.md](scripts/release/README.md) for accepted note types.
Enable local enforcement with `node scripts/commit/install.mjs`; CI validates
new commits too. Preserve this section when amending, squashing or merging.
