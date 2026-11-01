# Release notes

The notes on a release page, and the "What's New" the app shows once after an
update, come from two parts:

1. a hand-written note, `docs/release-notes/<tag>.md`, if there is one for the
   tag (`v0.9.4.md`, `v0.10.0-pre.1.md`), and
2. the list git-cliff builds from the commit subjects since the previous tag
   (see `cliff.toml` and the commit conventions in
   [`docs/release-process.md`](../release-process.md)).

The release workflow puts the note above the list. It reads the note from the
tagged commit, so the file has to be committed before the tag, and its name
has to match the tag exactly. Without a note the release gets the generated
list alone, as it always did.

Both parts end up in `update.json` and are signed with it, so a typo cannot be
fixed after the tag without a new version. The app renders a small subset of
Markdown: headings, bullet lists and links. The sign job refuses notes larger
than 128 KiB; a note should be a fraction of that.

## A release

Written for the people who use the app, not for developers, in German and in
English. Three parts, then the generated list:

```markdown
## What you will notice

- At most five points. Every action with the word on the button and its
  shortcut.

## What stays the same

- Vault files, KeePassXC, settings, Touch ID. Say so even when it seems
  obvious; it is the first thing people ask.

## What changes

- Shortcuts and behaviour that are different now, and anything the app did
  before and no longer does, one sentence each.
```

The technical list follows on its own and needs nothing from the note.

## A test build

Tags with a suffix (`v0.10.0-pre.N`) go to a small test group, never to
everyone. Their note is short and written in German, the test group's
language, even while the app itself is in English:

```markdown
## What is new

## What to try

## What is known to be broken
```
