# 0004 — a swallowed failure reported an absence of problems

- **Date discovered:** 2026-08-22
- **Introduced:** [`3a5025efe`](https://github.com/ste-bah/archon-cli/commit/3a5025efe) — 2026-04-16, TASK-TUI-303, the commit that created the gate
- **Fixed:** this change
- **Exposure:** 128 days, in the one duplication gate that can fail the build
- **Defect class:** [**vacuous check**](../defensive-patterns.md#dp-0--a-check-whose-scan-target-can-vanish-must-fail-not-pass) — an absence of findings reported as an absence of problems
- **Decision record:** [Rejected: raise the TUI duplication threshold](../decisions/rejected/2026-08-22-raise-the-tui-duplication-threshold.md)

## What the gate was for

`scripts/check-tui-duplication.sh` runs jscpd over `crates/archon-tui/src` and
fails if duplication exceeds 5% (NFR-TUI-MOD-003, AC-MOD-04). It runs in
`.github/workflows/ci.yml` as the `tui-duplication-check` job, which has **no**
`continue-on-error` — so it is the duplication gate that can actually fail a
build.

## What actually happened

The script parsed jscpd's JSON report, and then ended like this:

```bash
else
  printf 'TuiDuplicationGuard: no report file generated\n'
fi

printf 'TuiDuplicationGuard: PASS\n'
exit 0
```

The `PASS` is outside the `if`. Every path reached it.

That gave the gate two ways to report a clean tree without measuring one:

**1. No report at all.** If jscpd exited 0 but wrote nothing — a changed output
path, a flag the pinned version stopped accepting, a package fetch that resolved
to something unexpected — the script printed `no report file generated` and then
`PASS`, exit 0. The note went to stdout in a passing CI job, where it reads as
routine.

**2. A report covering nothing.** This is the worse one, because it produces a
number. jscpd over an empty or unreadable source directory writes a valid report
whose total is 0% duplicated across 0 files. The gate printed:

```
TuiDuplicationGuard: duplication = 0.00% (threshold = 5%)
TuiDuplicationGuard: PASS
```

Reproduced on the tree as it stood, with `TUI_SRC` pointed at an empty directory:
exit **0**. Not a warning, not an anomaly — a confident measurement of zero
duplication over nothing at all, in the format a reader trusts.

There was a third swallow feeding both:

```bash
DUP_PCT=$(python3 -c "..." 2>/dev/null || echo "unknown")
```

A missing `python3`, a truncated report, or a schema change became the string
`unknown`, which was printed into the percentage slot and then ignored.

**The absence of a finding was being reported as the absence of a problem.** Those
are different facts, and every one of the incidents in this series is a case of
the first being presented as the second.

## Why nothing caught it

There is a self-test, `scripts/check-tui-duplication-gate.selftest.sh`, added the
same day as the gate. Its own header states its scope:

```bash
# Injection test: proves check-tui-duplication.sh exits 1 when duplication > 5%.
```

It builds a fixture with two 30-line identical blocks and asserts the gate exits
non-zero. That is a real test and it works. But it only ever proved the gate fires
when it finds too much. It said nothing about the other way a gate is wrong —
finding nothing because it looked at nothing — which is precisely the half that
was broken, and precisely the half that
[postmortem 0001](0001-arch-lint-inspected-nothing-and-reported-green.md) is also
about. Two independent gates in this repository, written months apart, were each
guarded by a test that could not distinguish a clean scan from an empty one.

## The contrast is inside the repository

There are two duplication gates over the same directory, and they disagree about
what a missing report means.

`scripts/ci/check-duplicate-code.sh` gets it right:

```bash
if [[ ! -f "$REPORT_JSON" ]]; then
    echo "ERROR: jscpd report missing at $REPORT_JSON" >&2
    echo "jscpd exit $JSCPD_RC; stdout:" >&2
    tail -20 /tmp/jscpd-stdout.log >&2 || true
    exit 2
fi
```

Run side by side on the same tree at the same moment, they gave opposite verdicts:
`check-duplicate-code.sh` exited **2** with `ERROR: failed to parse jscpd report`,
while `check-tui-duplication.sh` exited **0** with `PASS`.

And the wiring inverts which one matters:

| Script | Missing report | Workflow / job | Can fail the build |
|---|---|---|---|
| `scripts/check-tui-duplication.sh` | printed `PASS`, exit 0 | `ci.yml` → `tui-duplication-check` | **yes** |
| `scripts/ci/check-duplicate-code.sh` | `exit 2` with the reason | `tui-observability.yml` → `tui-lint-duplication` | no — `continue-on-error: true` |

The gate that enforced was the vacuous one. The gate that told the truth was
advisory. Nobody chose that; it is what two independently written scripts and two
workflow files add up to when nothing checks the combination.

## The fix

The gate now refuses to say PASS unless it can show what it measured:

- A missing report is a failure, with the reason, on stderr.
- The report is parsed **once**, and that parse must succeed — no `|| echo
  "unknown"` fallback. A missing `python3` or a schema change now aborts under
  `set -e` instead of degrading into a printed word.
- **The denominator is asserted and printed.** A report covering zero files or
  zero lines fails. A passing run now says what it looked at:

```
TuiDuplicationGuard: duplication = 1.13% over 147 files / 34719 lines (threshold = 5%)
TuiDuplicationGuard: PASS
```

The self-test gained the case it never had: run the gate against an empty scan
target and require a non-zero exit *and* a stated reason. Against the old script
that case exits 0 and prints `PASS`, so the test is load-bearing rather than
decorative.

Both self-test cases pass: the original threshold injection, and the new vacuity
check.

## Other instances of the same shape in this tree

Recorded rather than fixed here, so they are not rediscovered from scratch:

- **`.archon/agents/coding-pipeline/*.md`** — the live ones. Fenced `bash` blocks
  in agent system prompts, under headings marked `MANDATORY`, containing
  `TYPE_ERRORS=$(grep -c "error TS" /tmp/phase6-typecheck.txt || echo "0")` and
  siblings in `code-quality-improver.md`, `final-refactorer.md`,
  `phase-6-reviewer.md` and `regression-detector.md`. If the `npm run typecheck`
  that was supposed to produce that file crashed, the file is empty or absent,
  `grep -c` finds nothing, and the agent reports zero type errors and declares the
  quality pass complete. A model reading those instructions will run them.
- **`.archon/agents/github/pr-manager.md`** — `gh pr checks || echo 'No PR checks
  available'`, hardened in this change. It is worth being precise about what that
  fix is worth: the `hooks:` block in agent frontmatter is **never executed**.
  `crates/archon-core/src/agents/loader/flat_file.rs` reads named keys out of the
  YAML by hand and hardcodes `hooks: None` at line 147, so the block is dropped on
  the floor. The file is vendored from claude-flow (its examples still say
  `owner: "ruvnet"`, `repo: "ruv-FANN"`). The change removes a misleading worked
  example from a tracked file that agents and humans read; it does not fix a
  running check, and it should not be counted as one.

  For the record, `gh pr checks` exits 0 **only** when every reported check
  passed. Measured against this repository: exit 1 on failing checks (PR #211,
  the PR from [postmortem 0003](0003-a-cfg-unix-test-was-cleared-by-a-windows-only-verification.md)),
  exit 8 while pending (PR #212), exit 1 for "no checks reported" (PR #5), exit 1
  for a PR that does not exist, exit 127 when `gh` is not on `PATH` — which is the
  normal state in Git Bash on this machine, where `gh` lives only inside WSL. So
  `|| echo 'No PR checks available'` printed that sentence in every state **except**
  the all-green one, which is the only state in which it was suppressed.

## Rules this produced

- [DP-13 — judge by exit code, never by counting matches in output](../defensive-patterns.md#dp-13--judge-by-exit-code-never-by-counting-matches-in-output)
- [DP-15 — never turn a tool failure into a benign message](../defensive-patterns.md#dp-15--never-turn-a-tool-failure-into-a-benign-message)
- and it is a second instance of [DP-2](../defensive-patterns.md#dp-2--every-scanner-reports-what-it-inspected-and-its-test-asserts-the-count-is-non-zero),
  which [0001](0001-arch-lint-inspected-nothing-and-reported-green.md) produced:
  report the denominator, and have the guarding test assert it is non-zero.
