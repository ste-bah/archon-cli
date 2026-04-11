# Cargo Test Baseline Snapshot

This directory holds a deterministic, normalized snapshot of the archon-cli
workspace's `cargo test` state at a known-good reference point. Phase-1..9
refactor tasks use it as a **regression guard**: any change that alters the
test inventory or the pass/fail/ignored tallies must be explained.

## Files

- `cargo_test_list.txt` — sorted, unique test names across all 14 workspace
  crates. One name per line, ANSI/timings/abs-paths stripped.
- `cargo_test_summary.txt` — exactly one line of the form
  `passed=N failed=M ignored=K` (plus ` timeout=T` if any crate timed out).

## What "baseline" means

The baseline records **reality, not aspiration**. Failing tests are NOT fixed
before capture — they are recorded as failing. A later task fixing the failure
is the moment to regenerate this baseline.

## When to regenerate

ONLY regenerate when **all three** conditions hold:

1. You are on `main` (not a feature branch).
2. An approved fix or refactor has legitimately changed test inventory/results.
3. You have explicit reviewer sign-off to update the baseline.

Never regenerate to "make CI green." Fix the regression instead.

## How to regenerate

```bash
bash scripts/regen-baseline.sh
```

The script is deterministic — running it twice must produce byte-identical
fixture files. If it does not, the normalization in the script is incomplete
and must be fixed before committing.

## How to diff in CI

Phase-1..9 tasks should run their own equivalent capture and diff against the
committed files:

```bash
bash scripts/regen-baseline.sh
diff -u tests/fixtures/baseline/cargo_test_list.txt <(git show HEAD:tests/fixtures/baseline/cargo_test_list.txt)
diff -u tests/fixtures/baseline/cargo_test_summary.txt <(git show HEAD:tests/fixtures/baseline/cargo_test_summary.txt)
```

A non-empty diff = regression. Investigate before merging.

## Cargo safety constraint (why the script looks paranoid)

This workspace has crashed WSL2 under unconstrained parallel cargo. The
regen script therefore:

- Uses `--jobs 1` (one rustc at a time)
- Uses `-- --test-threads=1` (one test runner at a time)
- Iterates crates sequentially with `-p <crate>` — never `--workspace`
- Wraps each crate in a 600s `timeout`; crates that exceed it are counted
  as `timeout=T` and skipped, not retried

Do not "optimize" these constraints away.

## Phase-4/6/7 inventory baselines (TASK-AGS-012)

Three additional snapshots live beside the cargo-test baseline. They
are NOT run by `regen-baseline.sh` — they have their own capture
script and a different regeneration cadence.

- `main_rs_loc.txt` — single LoC number for `src/main.rs` (REQ-FOR-D4).
  Phase-4 TUI modularization tasks diff against this to prove they
  have actually shrunk main.rs.
- `slash_commands.txt` — sorted, unique slash literals found in match
  arms of `src/main.rs` (REQ-FOR-D7). Phase-7 slash-command coverage
  tasks assert their new list is a strict superset of this baseline.
- `providers.txt` — sorted concrete types that `impl LlmProvider for`
  under `crates/archon-llm/src/providers/` (REQ-FOR-D6). Phase-6
  provider breadth tasks likewise assert superset.

Regenerate with:

```bash
bash scripts/capture-inventory.sh
```

The script is deterministic (`LC_ALL=C sort`) and idempotent at a
fixed git rev — running it twice at the same SHA produces byte-
identical files. Every output file has a header comment
`# captured <YYYY-MM-DD> from git rev <SHA>` so drift is traceable.

**Who regenerates:** the person advancing the D4/D6/D7 gauges. The new
values get committed as a progress milestone alongside the code that
justifies them. Phase-0 does NOT assert superset yet — that assertion
is unlocked in phase-4/6/7 by extending
`crates/archon-core/tests/baseline_inventory_superset.rs` (a
placeholder no-op test lives there today).

