#!/usr/bin/env bash
# check-tui-coverage.sh
# Line coverage gate for archon-tui using cargo llvm-cov.
#
# NFR-TUI-QUAL-002: archon-tui must maintain >= TUI_COVERAGE_MIN line coverage.
# Default TUI_COVERAGE_MIN is 10 (skeleton floor). TASK-TUI-330 raises to 80.
#
# Usage:   bash scripts/check-tui-coverage.sh
# Env:     TUI_COVERAGE_MIN (int, default 10)
set -euo pipefail

THRESHOLD="${TUI_COVERAGE_MIN:-10}"
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