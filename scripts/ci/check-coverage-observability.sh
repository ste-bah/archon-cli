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

# NOTE: --offline OMITTED here intentionally (#232). cargo-llvm-cov uses
# a separate target dir (target/llvm-cov-target/) NOT populated by
# Swatinem/rust-cache@v2, so cargo needs network access to resolve deps
# on the first invocation. Developer-invoked cargo commands MUST keep
# --offline per the WSL2 dev-box rule; this loosening is CI-runner-
# specific only because GitHub-hosted runners have network access.
cargo llvm-cov -p archon-observability -j1 --fail-under-lines "$THRESHOLD" --summary-only -- --test-threads=2

echo "OK: coverage >= ${THRESHOLD}%"
