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
# Every `cargo test` invocation across the workspace runs with
# `--test-threads=2`. This is enforced here because:
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
    cargo clippy --workspace --all-targets -- -D warnings
fi

# ---------------------------------------------------------------------
# Step 5 — cargo test (ENFORCED --test-threads=2)
# ---------------------------------------------------------------------
if should_run "test"; then
    banner 5 "cargo test --test-threads=2"
    # The `--` separator passes --test-threads to each test binary; the
    # literal is visible in `bash -x` traces (validation criterion #3).
    cargo test --workspace --no-fail-fast -- --test-threads=2
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
    cargo test --workspace --no-fail-fast -- --list --format=terse \
        2>/dev/null | sort -u > "$TMPLIST" || true
    # A test may only be ADDED, never silently removed. `comm -23` gives
    # lines in baseline that are NOT in the current list — those are the
    # removals we must fail on.
    REMOVED="$(comm -23 <(sort -u "$BASELINE") <(sort -u "$TMPLIST") || true)"
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
        cargo bench -p archon-bench --no-run
    fi
fi

printf "${C_OK}== ci-gate: ALL STEPS PASSED ==${C_OFF}\n"
