# Contributing

Use Node.js 24 and enable the repository-local commit rules after cloning:

```sh
node scripts/commit/install.mjs
```

This installs a `commit-msg` hook and commit template through local Git config.
It is idempotent and refuses to replace existing custom hook/template settings;
when using other hook tooling, invoke `node scripts/commit/check.mjs --message-file "$1"`
from your existing commit-msg hook instead. Node must be available to the Git client.

Every commit, including merges, reverts and internal maintenance, needs exactly
one `Release Notes:` section outside code fences. Example for internal work:

```text
chore: update CI tooling

Release Notes:
- N/A
```

For user-facing changes, use the English bullets documented in
[scripts/release/README.md](scripts/release/README.md). Empty sections, placeholder
bullets, duplicate sections and mixing N/A with actual notes are rejected.
The template intentionally contains a placeholder instead of silently choosing N/A.
Preserve the notes in GitHub squash/merge messages as well.

CI checks every commit in the push before..after or PR base..head range, using
the PR head rather than GitHub's synthetic test merge. Existing history is not
rewritten or retroactively checked. Manual runs and new-branch pushes check the
current tip. Missing history fails validation. This adds no job or network request.
Local hooks can be bypassed, so require the existing **CI result** status in branch
protection to prevent merging commits that fail the rule. The repository workflow
does not change GitHub branch protection or block pushes at the server.
