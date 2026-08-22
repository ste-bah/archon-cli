# 0001 — arch-lint inspected nothing in two of three rules and reported green

- **Date discovered:** 2026-08-22
- **Introduced:** [`bddae1ddd`](https://github.com/ste-bah/archon-cli/commit/bddae1ddd) — 2026-04-12, the commit that activated the three rules (TASK-AGS-110)
- **Fixed:** [`bed66e1a0`](https://github.com/ste-bah/archon-cli/commit/bed66e1a0) — 2026-08-22
- **Exposure:** rule 3 vacuous from birth (132 days); rule 1 vacuous from 2026-04-16 (128 days). Green CI throughout, on a lint that was checking one rule out of three.
- **Defect class:** [**vacuous check**](../defensive-patterns.md#dp-0--a-check-whose-scan-target-can-vanish-must-fail-not-pass) — a check whose scan target can vanish must fail, not pass
- **Decision record:** [Rejected: restore the `BEGIN/END INPUT_HANDLER` markers](../decisions/rejected/2026-08-22-restore-the-input-handler-markers.md)

## What the check was for

`scripts/lint/arch-lint.sh` enforces the three D10 rules from
[`docs/architecture/spawn-everything-philosophy.md`](../architecture/spawn-everything-philosophy.md).
The one that matters most is rule 1: no `.process_message().await` on the
interactive input path. One synchronous agent call there parks the whole event
loop for the length of a turn — the original smoking gun the philosophy was
written against. It runs on every PR as the `arch-lint` job in
`.github/workflows/ci.yml`.

## What actually happened

**Rule 1 scoped itself to a region that did not exist.** It scoped to the lines
between `BEGIN INPUT_HANDLER` and `END INPUT_HANDLER` markers in `src/main.rs`,
and when it could not find them:

```bash
else
    echo "arch-lint: WARNING — BEGIN/END INPUT_HANDLER markers not found in src/main.rs" >&2
    # No markers = can't scope; warn but don't fail (markers might be in transit)
fi
```

The markers were real once. Their history is the whole story:

| Commit | Date | What happened |
|---|---|---|
| `bddae1ddd` | 2026-04-12 | Markers added at `src/main.rs:3323` / `:3921`, and the three rules activated against them (TASK-AGS-110). |
| `5f3828a77` | **2026-04-16** | `refactor(cli): extract run_interactive_session to session module`. The markers moved with the code, to `src/session.rs`. **Rule 1 went vacuous here** — the lint hardcodes `src/main.rs`, so it stopped seeing them the moment they left that file. |
| `2f5ee10b5` | 2026-04-24 | Markers moved again, to `src/session_loop/mod.rs`. |
| `0e06d738c` | 2026-05-24 | `Split session loop dispatch paths`. Markers deleted outright. |

Four days. The rule was activated on 2026-04-12 and stopped inspecting anything on
2026-04-16, and neither the refactor that moved the markers nor the one that
deleted them produced a single red build. Nobody did anything wrong in those two
commits — the marker text (`// BEGIN INPUT_HANDLER — arch-lint.sh scopes D1 grep
to this region`) travelled correctly with the code it annotated. It was the lint
that could not follow.

By the time this was found, `src/main.rs` had shrunk to a 113-line argument
dispatcher and the input handler lived in `src/session_loop/` and
`crates/archon-tui/src/event_loop/`.

The repo half-knew. `tests/tc_arch_02_grep_input_handler.rs` carried
`input_handler_markers_exist`, asserting the markers are present — `#[ignore]`d,
deferred to "AGS-106/107", "tracked under #224". There is no issue #224 in this
repository — the numbering has not reached it. So the one test that would have
caught the drift was disabled, waiting on a ticket that does not exist, and its
presence in the file made the gap look managed.

**Rule 3 grepped two files for functions neither contained.**

```bash
RULE3_FN_PATTERN='fn[[:space:]]+(handle_.*_input|on_key|process_key)[[:space:]]*\('
RULE3_PATHS=(src/main.rs crates/archon-tui/src/app.rs)
```

Both files exist. Both match that pattern **zero** times — and did so at
`bddae1ddd`, the activation commit itself. The convention was already
`handle_key_event`, `dispatch_user_prompt`, `dispatch_terminal_event`. Rule 3's
loop body never executed once in 132 days.

It also had `if [[ ! -f "${file}" ]]; then continue; fi`, so a deleted file would
have been skipped in silence too — a second vacuity path in the same eight lines.

**Rule 2 was the only rule doing any work,** and it happened to be the one written
as a positive assertion — "this exact awaited send must be present" — which cannot
go vacuous the same way, because absence is the failure.

The script then printed `arch-lint: all checks passed` and exited 0.

## Why nothing caught it

The guarding test asserted the exit code and only the exit code:

```rust
assert!(
    output.status.success(),
    "TC-ARCH-02: arch-lint.sh exited with non-zero on clean tree. ..."
);
```

That assertion is true of a working lint and equally true of a lint that reads no
files. **Exit 0 from a scanner is a statement about violations found, not about
work done**, and the two are indistinguishable from outside unless the scanner says
how much it looked at. A test that can only observe the exit code cannot tell a
clean tree from an empty scan, so it will report the same green for both — forever,
because nothing degrades and nothing gets slower.

The WARNING line on stderr was, in principle, the signal. In practice it went to
stderr in a passing CI job, where nobody reads it.

## The fix

Every rule now declares its scan target, refuses to pass if that target is empty,
and prints what it inspected.

```bash
vacuous() {
    echo "arch-lint: RULE ${1} HAS NOTHING TO SCAN — ${2}" >&2
    echo "arch-lint: re-point the rule at where the code lives now; a rule that" >&2
    echo "           inspects nothing must never report success." >&2
    exit 1
}
```

Rule 1 checks that each region directory exists, that it yields `.rs` sources, that
`spawn_turn` still appears in it (the anchor proving it is still the code the rule
is about), and that there is at least one `.await` for the rule to clear. Rule 2
checks its source file exists. Rule 3 fails if no function matches the naming
convention. Each prints a line:

```
arch-lint: rule=1 files=24 sites=226 name=no .await on agent work in input handler (D1)
arch-lint: rule=2 files=1  sites=1   name=Agent event transport must await bounded capacity (D3)
arch-lint: rule=3 files=24 sites=30  name=no .await on agent work in input handler function (D1 broad)
```

and `tests/tc_arch_02_grep_input_handler.rs` now asserts, for each rule in
`EXPECTED_RULES = [1, 2, 3]`, that the line is present and that `files` and `sites`
are both greater than zero. Adding a rule to the script without adding it to that
list leaves it unasserted; removing one from the script fails the test loudly.

The region itself stopped being a comment pair and became a directory list, for the
reasons in the [decision record](../decisions/rejected/2026-08-22-restore-the-input-handler-markers.md).

## Rules this produced

- [DP-1 — a lint must verify its scan target resolves before reporting on it](../defensive-patterns.md#dp-1--a-lint-must-verify-its-scan-target-resolves-before-reporting-on-it)
- [DP-2 — every scanner reports what it inspected, and its test asserts the count is non-zero](../defensive-patterns.md#dp-2--every-scanner-reports-what-it-inspected-and-its-test-asserts-the-count-is-non-zero)
- [DP-3 — anchor a scan to a structural target, not to a comment](../defensive-patterns.md#dp-3--anchor-a-scan-to-a-structural-target-not-to-a-comment)
- [DP-4 — an `#[ignore]`d test is not a plan](../defensive-patterns.md#dp-4--an-ignored-test-is-not-a-plan)
