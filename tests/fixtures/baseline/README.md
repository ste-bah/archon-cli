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
