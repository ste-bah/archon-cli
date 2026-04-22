#!/usr/bin/env bash
# check-coverage.sh — enforce AC-OBSERVABILITY-04 / NFR-TUI-QUAL-003.
#
# Runs cargo llvm-cov against the archon-tui crate and fails the build if line
# coverage is below $COVERAGE_THRESHOLD (default 80). Spec reference:
# TECH-TUI-OBSERVABILITY line 1134 — `cargo llvm-cov --package archon-tui
# --fail-under-lines 80`.
#
# Environment overrides:
#   COVERAGE_THRESHOLD  integer line-coverage percentage (default 80)
set -euo pipefail

THRESHOLD="${COVERAGE_THRESHOLD:-80}"

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "ERROR: cargo-llvm-cov not installed (run 'cargo install cargo-llvm-cov')" >&2
    exit 2
fi

cargo llvm-cov --package archon-tui --fail-under-lines "$THRESHOLD" --summary-only

echo "OK: coverage >= ${THRESHOLD}%"
