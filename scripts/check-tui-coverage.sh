#!/usr/bin/env bash
# check-tui-coverage.sh
# Line coverage gate for archon-tui using cargo llvm-cov.
#
# NFR-TUI-QUAL-002 / NFR-TUI-QUAL-003: archon-tui must maintain >= 80% line
# coverage. TASK-TUI-328 ratcheted this from the 10% skeleton floor to the
# 80% production gate after TASK-TUI-305..327 filled in unit coverage.
#
# The threshold is intentionally hardcoded: TUI-328 removed the
# TUI_COVERAGE_MIN env override so CI cannot silently drift below the gate.
# If coverage legitimately needs to change, update this constant AND
# docs/tui-coverage-baseline.json in the same commit.
#
# Usage: bash scripts/check-tui-coverage.sh
set -euo pipefail

THRESHOLD=80
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "=== tui-coverage gate ==="
echo "Threshold: ${THRESHOLD}% line coverage"

# Verify cargo-llvm-cov is installed before running tests.
if ! cargo llvm-cov --version >/dev/null 2>&1; then
    echo "ERROR: cargo-llvm-cov is not installed."
    echo "::error::check-tui-coverage: cargo-llvm-cov not found. Run: cargo install cargo-llvm-cov"
    echo "Install with: cargo install cargo-llvm-cov --locked"
    exit 1
fi

cd "$REPO_ROOT"

# Run coverage on archon-tui only (--workspace forbidden per ABSOLUTE CARGO RULES).
# -j1: ABSOLUTE CARGO RULES
# --package archon-tui: scope to archon-tui only
# --fail-under-lines: cargo llvm-cov flag (BEFORE --) to exit 1 if line coverage below threshold
# The -- separator passes args to cargo test; --test-threads is for the test runner.
cargo llvm-cov -j1 --package archon-tui --fail-under-lines "${THRESHOLD}" -- --test-threads=2
exit_code=$?

if [ $exit_code -eq 0 ]; then
    echo "PASS: archon-tui line coverage >= ${THRESHOLD}%"
else
    echo "FAIL: archon-tui line coverage < ${THRESHOLD}%"
    echo "::error::check-tui-coverage: line coverage below ${THRESHOLD}% threshold"
fi

exit $exit_code