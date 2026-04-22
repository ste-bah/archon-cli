#!/usr/bin/env bash
# check-coverage-observability.sh — enforce coverage gate for archon-observability.
#
# Runs cargo llvm-cov against the archon-observability crate and fails the build
# if line coverage is below $COVERAGE_THRESHOLD (default 80).
# Threshold chosen below current coverage (82.59% as of 2026-04-22 probe).
#
# Environment overrides:
#   COVERAGE_THRESHOLD  integer line-coverage percentage (default 80)
set -euo pipefail

THRESHOLD="${COVERAGE_THRESHOLD:-80}"

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "ERROR: cargo-llvm-cov not installed (run 'cargo install cargo-llvm-cov')" >&2
    exit 2
fi

cargo llvm-cov -p archon-observability -j1 --offline --fail-under-lines "$THRESHOLD" --summary-only -- --test-threads=2

echo "OK: coverage >= ${THRESHOLD}%"
