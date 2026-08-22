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
| [0004](0004-a-swallowed-failure-reported-an-absence-of-problems.md) | a swallowed failure reported an absence of problems | vacuous check (`PASS` printed outside the guard) | 128 days |
| [0005](0005-a-documented-page-was-never-committed-because-git-add-skips-ignored-paths.md) | a documented page was never committed because `git add` skips ignored paths | vacuous check (staging step skipped its subject) | 9 days, 3 dangling links |

## What they all have in common

> **A check whose scan target can vanish must fail, not pass.**

Every incident above is an instance. A lint scoped to a comment region that had
moved; a test whose subprocess never spawned; a test erased by `cfg` on the
platform that verified it; a duplication gate that printed `0.00%` and `PASS` over
zero files; a `git add` that silently skipped the file it was given. In each case
the step reported success, and in each case it reported success *because* its
subject was missing — the absence produced exactly the zero that means "clean".

The rule and its consequences are [DP-0](../defensive-patterns.md#dp-0--a-check-whose-scan-target-can-vanish-must-fail-not-pass).

Three of these ran green for over four months. None of them degraded, got slower,
or became flaky first. **This class of defect has no warning signs** — which is why
the countermeasure has to be structural rather than vigilant.

0005 is worth noting separately: it was found by the link checker added *in this
change*, on its first run, having gone unnoticed for nine days. That is the
argument for the checker in one sentence.

## The admission standard

**A numbered postmortem must be an incident in this codebase, verifiable from this
history.** Not "a true story about a real failure" — the file, the commit, the CI
run or the reproduction has to be something the next reader can go and look at.

This is the property that makes the series worth consulting. Every rule in
[`defensive-patterns.md`](../defensive-patterns.md) is enforced against people who
find it inconvenient, and it can only carry that weight if "read the postmortem"
leads to evidence rather than to an assertion.

The standard has already cost something, which is the point:

- **0004 was written twice.** The first version described an ad-hoc CI-watching
  script that reported ALL GREEN because `gh` was not on `PATH`. That genuinely
  happened, but the script lived outside the repository and no longer exists, so
  the writeup had to reconstruct its own evidence. It was rewritten around
  `scripts/check-tui-duplication.sh`, which is committed, runs in CI, and can be
  reproduced in one command.
- **[DP-14](../defensive-patterns.md#dp-14--retired) was retired with it**, because
  its only support was that unrecoverable script. The rule was sound. It still
  went, and the number is left as a gap rather than reused.

If an incident does not clear the bar, fold its rules into a note that does, or
leave them out. **A thin postmortem is worse than none** — it teaches the reader
that the citations are decorative.

## Writing one

`NNNN-short-slug.md`, next number in sequence. Front matter as a bullet list:
date discovered, the commit that introduced it, the commit that fixed it, exposure
window, defect class linked to its rule.

Then:

- **What the check was for** — the invariant, stated as if it had worked. A reader
  who does not know why the thing exists cannot judge the fix.
- **What actually happened** — with the real code, the real dates, the real commit
  SHAs. Quote the defective lines, and reproduce the failure if you can: 0004
  re-runs the old gate against an empty scan target and shows it printing
  `0.00%` and `PASS`. Guessing is worse than omitting.
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
