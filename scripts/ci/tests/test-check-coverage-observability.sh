#!/usr/bin/env bash
# Self-test for check-coverage-observability.sh (TASK-AGS-OBS-913).
#
# Case A: cargo llvm-cov --fail-under-lines 0 on hermetic fixture -> rc == 0.
# Case B: cargo llvm-cov --fail-under-lines 100 on hermetic fixture -> rc != 0
#         (fixture intentionally ships an uncovered `untested` fn so 100%
#         is unattainable).
#
# We invoke `cargo llvm-cov` directly inside a copy of the covergate/ fixture
# instead of running check-coverage-observability.sh itself, because the real
# script targets `-p archon-observability` in the parent workspace. The flag
# surface under test (--fail-under-lines + --summary-only) is identical, and
# this is the only way to get a hermetic, focused run.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECKER="${SCRIPT_DIR}/../check-coverage-observability.sh"
FIXTURE_SRC="${SCRIPT_DIR}/fixtures/covergate"

if [[ ! -x "$CHECKER" ]]; then
    echo "FAIL: check-coverage-observability.sh not executable at $CHECKER"
    exit 1
fi

if [[ ! -d "$FIXTURE_SRC" ]]; then
    echo "FAIL: fixture dir missing at $FIXTURE_SRC"
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "FAIL: cargo not available on PATH"
    exit 1
fi

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "FAIL: cargo-llvm-cov not installed (run 'cargo install cargo-llvm-cov')"
    exit 1
fi

WORK="$(mktemp -d -t covergate-obs-self-test-XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

cp -R "$FIXTURE_SRC" "$WORK/covergate"

run_llvm_cov() {
    local threshold="$1"
    (
        cd "$WORK/covergate"
        cargo llvm-cov -j1 --fail-under-lines "$threshold" --summary-only 2>&1
    )
}

# --- Case A: threshold 0 must always pass ---
echo "--- Case A: threshold 0 (expect rc == 0) ---"
set +e
OUT_A="$(run_llvm_cov 0)"
RC_A=$?
set -e
echo "$OUT_A" | tail -n 20
if [ "$RC_A" -ne 0 ]; then
    echo "TEST FAIL: threshold-0 rejected (rc=$RC_A, should have passed)"
    exit 1
fi
echo "PASS threshold-0-pass (rc=$RC_A)"

# --- Case B: threshold 100 must fail (untested() is uncovered) ---
echo "--- Case B: threshold 100 (expect rc != 0) ---"
set +e
OUT_B="$(run_llvm_cov 100)"
RC_B=$?
set -e
echo "$OUT_B" | tail -n 20
if [ "$RC_B" -eq 0 ]; then
    echo "TEST FAIL: threshold-100 accepted (rc=$RC_B, should have failed — fixture has uncovered code)"
    exit 1
fi
echo "PASS threshold-100-fails (rc=$RC_B)"

echo "ALL TESTS PASSED"
