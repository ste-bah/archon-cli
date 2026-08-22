# Postmortems

Numbered writeups of things that went wrong here. Each names a defect class, and
each defect class appears in [`docs/defensive-patterns.md`](../defensive-patterns.md)
as a rule stated for the next person — or agent — to read.

That pipeline is the practice: **incident → postmortem → stated rule → a document
agents actually consult.** A postmortem that names no rule is a story. A rule with
no postmortem behind it is an opinion, and gets relaxed the first time it is
inconvenient.

## Index

| # | Title | Defect class | Exposure |
|---|---|---|---|
| [0001](0001-arch-lint-inspected-nothing-and-reported-green.md) | arch-lint inspected nothing in two of three rules and reported green | vacuous check | 128–132 days |
| [0002](0002-a-test-passed-on-the-one-platform-where-it-could-not-run.md) | a test passed on the one platform where it could not run | vacuous check (subject never started) | 136 days |
| [0003](0003-a-cfg-unix-test-was-cleared-by-a-windows-only-verification.md) | a `#[cfg(unix)]` test was cleared by a Windows-only verification | verification blind to its subject | 2 hours, 1 pushed PR |
| [0004](0004-a-ci-watcher-reported-all-green-because-gh-was-not-on-path.md) | a CI watcher reported ALL GREEN because its tool was not on PATH | vacuous check (counted matches, not exit code) | 1 pushed PR |
| [0005](0005-a-documented-page-was-never-committed-because-git-add-skips-ignored-paths.md) | a documented page was never committed because `git add` skips ignored paths | vacuous check (staging step skipped its subject) | 9 days, 3 dangling links |

## What all four have in common

> **A check whose scan target can vanish must fail, not pass.**

Every incident above is an instance. A lint scoped to a deleted comment region; a
test whose subprocess never spawned; a test erased by `cfg` on the platform that
verified it; a watcher counting rows from a binary that was not on `PATH`; a
`git add` that silently skipped the file it was given. In each case the step
reported success, and in each case it reported success *because* its subject was
missing — the absence produced exactly the zero that means "clean".

The rule and its consequences are [DP-0](../defensive-patterns.md#dp-0--a-check-whose-scan-target-can-vanish-must-fail-not-pass).

Three of these ran green for over four months. None of them degraded, got slower,
or became flaky first. **This class of defect has no warning signs** — which is why
the countermeasure has to be structural rather than vigilant.

0005 is worth noting separately: it was found by the link checker added *in this
change*, on its first run, having gone unnoticed for nine days. That is the
argument for the checker in one sentence.

## Writing one

`NNNN-short-slug.md`, next number in sequence. Front matter as a bullet list:
date discovered, the commit that introduced it, the commit that fixed it, exposure
window, defect class linked to its rule.

Then:

- **What the check was for** — the invariant, stated as if it had worked. A reader
  who does not know why the thing exists cannot judge the fix.
- **What actually happened** — with the real code, the real dates, the real commit
  SHAs. Quote the defective lines. Guessing is worse than omitting: mark
  reconstructed detail as reconstructed, the way
  [0004](0004-a-ci-watcher-reported-all-green-because-gh-was-not-on-path.md) does.
- **Why nothing caught it** — usually the most valuable section, and usually about
  a test or gate that existed and was satisfied.
- **The fix**, and ideally the evidence it works: 0002's rewritten assertion went
  red on its first run after four months of green, which is the proof.
- **Rules this produced** — link each into `defensive-patterns.md`. If an incident
  produces no rule, say so and say why; not every incident generalises.

No blame. Every one of these was written by a competent person doing something
reasonable — 0001's markers were carried *correctly* through two refactors and the
lint still lost them. The interesting question is always what made the failure
invisible, not who typed it.

## See also

- [`docs/defensive-patterns.md`](../defensive-patterns.md) — the rules.
- [Decision records](../decisions/README.md) — including `rejected/`, where the
  fixes that were considered and turned down are recorded.
- [`CONTRIBUTING.md`](../../CONTRIBUTING.md) — the gates a change must pass.
