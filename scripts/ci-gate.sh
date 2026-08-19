#!/usr/bin/env bash
#
# scripts/ci-gate.sh — Archon-CLI CI orchestrator
#
# Runs every phase-0 guard in order and fails fast on the first failure.
# Single source of truth for CI; any GitHub Actions / GitLab / local hook
# should call this script rather than replicate its steps.
#
# Reference: TASK-AGS-007 (phase-0 prereqs)
#
# CARGO TEST THREAD POLICY
# ------------------------
# The CI gate's executable workspace test run uses `--test-threads=2`.
# Baseline discovery uses `--list`; regeneration has its own stricter policy.
# This is enforced here because:
#
#   1. REQ-FOR-D1/D2/D3 introduce shared global state (BACKGROUND_AGENTS
#      DashMap, task registry, tempdir-based .archon/) that deadlocks
#      under unlimited parallelism on WSL2 hosts.
#   2. Prior incidents (2026-04-11) crashed WSL2 when unlimited parallel
#      rustc+test processes saturated the kernel; `--test-threads=2` is
#      the project-wide safe floor.
#   3. Tests that need stricter isolation can opt into `#[serial_test::
#      serial]` individually; `--test-threads=2` is the default.
#
# See scripts/ci-gate.README.md for per-step rationale.

set -euo pipefail

# ---------------------------------------------------------------------
# Locate the repo root (one level up from scripts/).
# ---------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# ---------------------------------------------------------------------
# CLI flags: --only <step>, --skip-bench
# ---------------------------------------------------------------------
ONLY=""
SKIP_BENCH=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --only)
            ONLY="${2:-}"
            shift 2
            ;;
        --only=*)
            ONLY="${1#--only=}"
            shift
            ;;
        --skip-bench)
            SKIP_BENCH=1
            shift
            ;;
        -h|--help)
            sed -n '1,30p' "$0"
            exit 0
            ;;
        *)
            echo "ERROR: unknown flag: $1" >&2
            exit 2
            ;;
    esac
done

# ---------------------------------------------------------------------
# Colour banners. Disable if stdout is not a TTY (CI-friendly).
# ---------------------------------------------------------------------
if [[ -t 1 ]]; then
    C_BANNER='\033[1;36m'
    C_OK='\033[1;32m'
    C_FAIL='\033[1;31m'
    C_OFF='\033[0m'
else
    C_BANNER=''; C_OK=''; C_FAIL=''; C_OFF=''
fi

banner() {
    local num="$1"
    local name="$2"
    printf "${C_BANNER}== STEP %s: %s ==${C_OFF}\n" "$num" "$name"
}

should_run() {
    local key="$1"
    if [[ -z "$ONLY" ]]; then
        return 0
    fi
    if [[ "$ONLY" == "$key" ]]; then
        return 0
    fi
    return 1
}

# ---------------------------------------------------------------------
# Step 1 — FileSizeGuard (TASK-AGS-002)
# ---------------------------------------------------------------------
if should_run "file-sizes"; then
    banner 1 "FileSizeGuard (<=500 lines)"
    bash scripts/check-file-sizes.sh
fi

# ---------------------------------------------------------------------
# Step 2 — BannedImports (TASK-AGS-003)
# ---------------------------------------------------------------------
if should_run "banned-imports"; then
    banner 2 "BannedImports"
    bash scripts/check-banned-imports.sh
fi

# ---------------------------------------------------------------------
# Step 2b — R0 entry gate (learning prerequisite closure evidence)
#
# Re-verifies docs/development/r0-entry-gate.evidence: findings 9, 11, 17 and
# 40-43 of the 2026-07-11 core audit must still be closed in source,
# still be reached by a live call site, and still be covered by the named
# behavioural tests. The roadmap forbids promoting any behaviour-changing slice
# while this is red (docs/development/learning-roadmap-r1-r8-w5-w6.md line 35).
#
# Commit-ancestry checking is best-effort here and mandatory in CI: a local
# working copy is often a shallow or Windows-created worktree whose history the
# running shell cannot walk. The `r0-entry-gate` job in .github/workflows/ci.yml
# checks out with fetch-depth: 0 and passes --require-commits, so the skip
# cannot become the permanent state of the check.
# ---------------------------------------------------------------------
if should_run "r0-entry-gate"; then
    banner 2b "R0 entry gate (learning prerequisite closure)"
    bash scripts/check-r0-entry-gate.sh
fi

# ---------------------------------------------------------------------
# Step 2c — deferral markers (replaces two dozen per-task grep gates)
#
# `scripts/tests/` used to hold 24 per-task verifiers, each grepping one
# hardcoded path for one hardcoded `TODO(TASK-x)` string. They were deleted:
# everything else they checked is compiler-enforced, and the hardcoded paths
# meant they went red on file splits rather than on defects — all 8 that were
# failing when they were removed were false alarms of exactly that kind.
#
# The one thing they checked that a compiler cannot is whether a wired slice
# has regressed to a stub. That survives here, once, over the whole tree.
# ---------------------------------------------------------------------
if should_run "deferral-markers"; then
    banner 2c "deferral markers (allowlisted stubs only)"
    bash scripts/check-deferral-markers.sh --self-test
    bash scripts/check-deferral-markers.sh
fi

# ---------------------------------------------------------------------
# Step 2d — self-tests of the gates above
#
# A gate nobody has watched go red is indistinguishable from a gate that
# cannot go red, and a silently-permissive gate is worse than no gate: it
# reports GREEN for work it never checked. These prove the guards can fail.
# See scripts/tests/README.md for what is allowed to live there.
# ---------------------------------------------------------------------
if should_run "gate-self-tests"; then
    banner 2d "gate self-tests"
    bash scripts/tests/test_check_file_sizes.sh
    bash scripts/tests/test_ci_baseline_diff.sh
    bash scripts/tests/archon-init-test.sh
    bash scripts/tests/ci-preserve-invariants-wired.sh
fi

# ---------------------------------------------------------------------
# Step 3 — cargo fmt --check
# ---------------------------------------------------------------------
if should_run "fmt"; then
    banner 3 "cargo fmt --check"
    cargo fmt --all -- --check
fi

# ---------------------------------------------------------------------
# Step 4 — cargo clippy (-D warnings)
# ---------------------------------------------------------------------
if should_run "clippy"; then
    banner 4 "cargo clippy"
    cargo clippy --workspace --all-targets --jobs 1 -- -D warnings
fi

# ---------------------------------------------------------------------
# Step 5 — cargo test (ENFORCED --test-threads=2)
# ---------------------------------------------------------------------
if should_run "test"; then
    banner 5 "cargo test --test-threads=2"
    # The `--` separator passes --test-threads to each test binary; the
    # literal is visible in `bash -x` traces (validation criterion #3).
    cargo test --workspace --jobs 1 --no-fail-fast -- --test-threads=2
fi

# ---------------------------------------------------------------------
# Step 6 — baseline test-list diff (TASK-AGS-001)
# ---------------------------------------------------------------------
if should_run "baseline-diff"; then
    banner 6 "cargo test --list vs tests/fixtures/baseline/cargo_test_list.txt"
    BASELINE="tests/fixtures/baseline/cargo_test_list.txt"
    if [[ ! -f "$BASELINE" ]]; then
        printf "${C_FAIL}ERROR: baseline file missing: %s${C_OFF}\n" "$BASELINE"
        exit 1
    fi
    TMPLIST="$(mktemp)"
    trap 'rm -f "$TMPLIST"' EXIT

    if bash scripts/list-cargo-tests.sh "$TMPLIST"; then
        :
    else
        discovery_rc=$?
        printf "${C_FAIL}ERROR: cargo test discovery failed${C_OFF}\n" >&2
        exit "$discovery_rc"
    fi
    # Compare full normalized identities. Approved moves and renames are
    # reviewed into the baseline explicitly so an unrelated same-named test
    # cannot mask deletion of the protected test.
    REMOVED="$(comm -23 \
        <(LC_ALL=C sort -u "$BASELINE") \
        <(LC_ALL=C sort -u "$TMPLIST"))"
    if [[ -n "$REMOVED" ]]; then
        printf "${C_FAIL}ERROR: tests were removed from the baseline:${C_OFF}\n%s\n" "$REMOVED"
        exit 1
    fi
fi

# ---------------------------------------------------------------------
# Step 7 — cargo bench --no-run (TASK-AGS-005)
# ---------------------------------------------------------------------
if should_run "bench"; then
    if [[ "$SKIP_BENCH" -eq 1 ]]; then
        printf "${C_BANNER}== STEP 7: bench SKIPPED (--skip-bench) ==${C_OFF}\n"
    else
        banner 7 "cargo bench -p archon-bench --no-run"
        cargo bench -p archon-bench --jobs 1 --no-run
    fi
fi

printf "${C_OK}== ci-gate: ALL STEPS PASSED ==${C_OFF}\n"
