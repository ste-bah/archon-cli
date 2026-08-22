# 0005 — a documented page was never committed because `git add` skips ignored paths

- **Date discovered:** 2026-08-22, by the link checker added alongside this postmortem
- **Introduced:** [`b722e88ac`](https://github.com/ste-bah/archon-cli/commit/b722e88ac) — 2026-08-13
- **Fixed:** this change — `.gitignore` exceptions plus `tests/docs_cross_references.rs`
- **Exposure:** 9 days, three dangling links on the published documentation tree
- **Defect class:** [**vacuous check**](../defensive-patterns.md#dp-0--a-check-whose-scan-target-can-vanish-must-fail-not-pass) — a staging step that skipped its subject and exited 0

## What happened

`b722e88ac` ("fix(llm): make prompt caching work on every provider, and price it
honestly") states in its own message:

> Docs: `docs/providers/bedrock.md` and `docs/reference/prompt-caching.md`, both
> linked from `docs/README.md`.

Its diff contains four documentation files. `docs/providers/bedrock.md` is not one
of them. The file has never existed in this repository's history, on any branch.

Three committed pages link to it: `docs/README.md`, `docs/reference/prompt-caching.md`
and `docs/release-notes/v1.9.1.md`. All three have been shipping a 404 since
2026-08-13.

## Why it was not committed

`.gitignore` excludes the whole documentation tree and re-includes it directory by
directory:

```gitignore
/docs/*
!/docs/architecture/
!/docs/architecture/**
...
!/docs/reference/
!/docs/reference/**
```

`/docs/providers/` was never on that list. The provider pages that *are* tracked —
`codex.md`, `runtime.md`, `cloud-and-local.md` and the rest — are tracked only
because they predate the rule. Any new file in that directory is invisible to
`git add`.

And `git add` says nothing about it. Staging a path that matches an ignore rule is
a silent no-op with exit status 0. `git commit` then succeeds, on a smaller diff
than the author intended, and the commit message describes a file that is sitting
untracked in the working tree looking exactly like a file that was committed.

The same is true of `git add docs/` and `git add -A`. Nothing in the normal flow
distinguishes "added" from "skipped because ignored" — you have to ask, with
`git status --ignored` or `git check-ignore`, and nobody asks about a file they
just wrote.

## Why nothing caught it

Nothing was looking. Until this change there was no check that a link in a
committed markdown file resolves to a committed file.

The most striking part is that this repository had already been bitten by the exact
same rule and had documented it — in `.gitignore` itself, three lines above the
gap:

```gitignore
# Exception: screenshots referenced by the committed docs. Without this the
# markdown ships with fifteen image links that 404, because `git add` skips
# ignored paths silently and nothing warns that a tracked page points at an
# untracked file.
!/docs/images/
!/docs/images/**
```

Fifteen broken image links, diagnosed correctly, root cause understood, written
down at the point of the fix — and the fix was one more exception line. The
*mechanism* was left in place, so it fired again nine months later on a different
directory. **A comment explaining a hazard is not a guard against it.** The next
person to add a directory under `docs/` will hit it a third time unless something
fails.

`docs/providers/` is not the only gap: `docs/agents/`, `docs/learning/`,
`docs/security/`, `docs/generated/` and `docs/providers/` are all tracked-by-history
rather than tracked-by-rule, and new files in any of them will be skipped in
silence.

## The fix

1. **The missing exceptions.** `/docs/providers/` and the new written-practice
   tree (`/docs/defensive-patterns.md`, `/docs/postmortem/`, `/docs/decisions/`)
   are re-included, each with a comment saying why.
2. **`docs/providers/bedrock.md` was written**, from the actual Bedrock code and
   config rather than from the README's description of it. Writing it surfaced
   three further claims in the `docs/README.md` entry that the code does not
   support; those are recorded in the doc rather than repeated.
3. **`tests/docs_cross_references.rs`** now fails if any relative link in a
   documentation page points at a file that does not exist **or that git does not
   track**. The second half is the one that catches this defect class: an ignored
   file is present on the author's disk and absent from every clone, so a
   disk-only check passes locally and the breakage ships.

## Rules this produced

- [DP-18 — a link in a committed document must point at a committed file](../defensive-patterns.md#dp-18--a-link-in-a-committed-document-must-point-at-a-committed-file)
- [DP-19 — writing the hazard down is not fixing it](../defensive-patterns.md#dp-19--writing-the-hazard-down-is-not-fixing-it)
