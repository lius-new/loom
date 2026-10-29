# Release notes and drafts

Write English release notes in the body of the commit that introduces a change.
For pull requests, write the same section in the PR description and preserve it
in the squash commit message. A conventional commit title is optional; the notes
generator reads the body, not the title.

```text
feat(editor): support selecting documentation text

Release Notes:

- Added text selection and copying in editor documentation cards.
- Improved wrapping of long diagnostic messages.
- Fixed completion labels drawing matched characters twice.
- Upgrade: Replace `ask` and `never` installation settings with `manual`.
```

Use `- N/A` for internal work with no user-facing changes. Use the exact
`Release Notes:` heading and top-level bullets starting with `Added`, `Improved`,
`Changed`, `Removed`, `Fixed`, `Upgrade:` or `Breaking change:`. Indent wrapped
continuation lines. Put Git trailers such as `Co-authored-by:` after the section. Write prose without
icons. Upgrade entries must describe the action users need to take.

Preview the next version without creating a tag or contacting GitHub:

```sh
node scripts/release/notes.mjs --version 0.1.3 --output target/release-notes.md
node --test scripts/release/release.test.mjs
```

The generator reads the complete commit bodies between the nearest reachable
application version tag and the selected commit. Tags for language packages and
language servers are excluded. It also works when the current application tag
does not exist yet. `- N/A` entries are omitted, identical entries are deduplicated,
and recorded changes are grouped under Added, Improved, Changed, Removed, Fixed
and Upgrade notes, with commit links and a full changelog link. Commits without
a section are listed as warnings and omitted; their titles are not guessed into
user-facing prose.
Malformed sections fail generation instead of silently dropping their contents.

## Publishing

1. Sync the application version in `Cargo.toml`, its `loom` entry in `Cargo.lock`,
   `LOOM_VERSION` in the CI workflow and the default version inputs in the four
   release workflows. Commit the changes, keeping any release entries in the
   commit body.
2. When ready to release, push the corresponding application tag. The separate
   **Prepare release draft** workflow generates a temporary notes file and creates
   a GitHub Release draft. It preserves the body if a draft already exists.
3. The Windows, Linux and macOS workflows build and upload their assets to that
   draft. Uploads wait up to five minutes for draft creation (or transient API
   recovery), and fail immediately on authentication errors or published releases.
   They never create another release or change its notes. If draft
   preparation fails, fix it and rerun that workflow before retrying uploads.
4. Review the English notes, build results and all platform packages on GitHub,
   then publish the draft manually. Nothing publishes the release automatically.

The draft workflow can also be run manually for an existing tag. Linux and macOS
manual packaging runs upload only with the `publish` input enabled; Windows
manual runs upload automatically. Uploads require the tag to match the build
commit and the release to remain a draft. A published release is left untouched.
The scripts use `--verify-tag` and never create or move tags.

No version-specific notes files are maintained in the repository. For v0.1.3,
the release preparation commit contains a one-time summary of changes since
v0.1.2, including the installation setting migration. Existing published commit
history is preserved. Subsequent versions collect notes from their own commits.
