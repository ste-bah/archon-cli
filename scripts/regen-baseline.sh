#!/usr/bin/env bash
# regen-baseline.sh — Capture a deterministic cargo test baseline snapshot.
#
# SAFETY: This workspace has crashed WSL2 when cargo runs unconstrained.
# Every cargo invocation in this script uses --jobs 1 and --test-threads=1,
# and iterates crates sequentially. Do NOT parallelize.
#
# Outputs (all normalized, deterministic):
#   tests/fixtures/baseline/cargo_test_list.txt     — sorted unique test names
#   tests/fixtures/baseline/cargo_test_summary.txt  — one line: passed=N failed=M ignored=K [timeout=T]
#
# Usage: bash scripts/regen-baseline.sh

set -uo pipefail

# Resolve repo root (script lives in scripts/)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

BASELINE_DIR="tests/fixtures/baseline"
mkdir -p "$BASELINE_DIR"

LIST_FILE="$BASELINE_DIR/cargo_test_list.txt"
SUMMARY_FILE="$BASELINE_DIR/cargo_test_summary.txt"

CRATES=(
  archon-consciousness
  archon-context
  archon-core
  archon-leann
  archon-llm
  archon-mcp
  archon-memory
  archon-permissions
  archon-pipeline
  archon-plugin
  archon-sdk
  archon-session
  archon-tools
  archon-tui
)

TMPDIR_RUN="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_RUN"' EXIT

# Normalization: strip ANSI, timings (1.23s), absolute /tmp and /home paths, PIDs, tempdir hashes.
normalize() {
  sed -r \
    -e 's/\x1b\[[0-9;]*[A-Za-z]//g' \
    -e 's/[0-9]+\.[0-9]+s//g' \
    -e 's#/tmp/[A-Za-z0-9_.\-]+##g' \
    -e 's#/home/[^ ]*##g' \
    -e 's/\bpid[ =:][0-9]+//gi' \
    -e 's/[[:space:]]+$//'
}

TOTAL_PASSED=0
TOTAL_FAILED=0
TOTAL_IGNORED=0
TOTAL_TIMEOUT=0

NAMES_AGGREGATE="$TMPDIR_RUN/names.raw"
: > "$NAMES_AGGREGATE"

echo "[regen-baseline] Starting capture across ${#CRATES[@]} crates..." >&2

for crate in "${CRATES[@]}"; do
  echo "[regen-baseline] ===== $crate =====" >&2

  RUN_LOG="$TMPDIR_RUN/run-$crate.log"
  LIST_LOG="$TMPDIR_RUN/list-$crate.log"

  # --- Pass 1: run tests, capture summary lines ---
  set +e
  timeout 600 cargo test -p "$crate" --no-fail-fast --jobs 1 -- --test-threads=1 \
    > "$RUN_LOG" 2>&1
  rc=$?
  set -e

  if [ $rc -eq 124 ]; then
    echo "[regen-baseline] $crate: TIMEOUT" >&2
    TOTAL_TIMEOUT=$((TOTAL_TIMEOUT + 1))
  fi

  # Parse every "test result: ok. N passed; M failed; ... K ignored; ..." line.
  # There is one per test binary in the crate.
  while IFS= read -r line; do
    p=$(echo "$line" | sed -nr 's/.*test result:[^0-9]*([0-9]+) passed.*/\1/p')
    f=$(echo "$line" | sed -nr 's/.*test result:[^;]*;[^0-9]*([0-9]+) failed.*/\1/p')
    i=$(echo "$line" | sed -nr 's/.*test result:[^;]*;[^;]*;[^0-9]*([0-9]+) ignored.*/\1/p')
    [ -n "$p" ] && TOTAL_PASSED=$((TOTAL_PASSED + p))
    [ -n "$f" ] && TOTAL_FAILED=$((TOTAL_FAILED + f))
    [ -n "$i" ] && TOTAL_IGNORED=$((TOTAL_IGNORED + i))
  done < <(grep -E '^test result:' "$RUN_LOG" || true)

  # --- Pass 2: list tests by name ---
  set +e
  timeout 600 cargo test -p "$crate" --no-fail-fast --jobs 1 -- --list --format=terse \
    > "$LIST_LOG" 2>&1
  set -e

  # Lines of form "test_name: test" — extract the name.
  grep -E ': test$' "$LIST_LOG" 2>/dev/null \
    | sed -r 's/: test$//' \
    | normalize \
    >> "$NAMES_AGGREGATE" || true
done

# --- Finalize list: normalize, sort -u, write ---
normalize < "$NAMES_AGGREGATE" \
  | grep -v '^[[:space:]]*$' \
  | LC_ALL=C sort -u \
  > "$LIST_FILE"

# --- Finalize summary ---
if [ "$TOTAL_TIMEOUT" -gt 0 ]; then
  printf 'passed=%d failed=%d ignored=%d timeout=%d\n' \
    "$TOTAL_PASSED" "$TOTAL_FAILED" "$TOTAL_IGNORED" "$TOTAL_TIMEOUT" \
    > "$SUMMARY_FILE"
else
  printf 'passed=%d failed=%d ignored=%d\n' \
    "$TOTAL_PASSED" "$TOTAL_FAILED" "$TOTAL_IGNORED" \
    > "$SUMMARY_FILE"
fi

echo "[regen-baseline] Done." >&2
echo "[regen-baseline] list:    $(wc -l < "$LIST_FILE") lines" >&2
echo "[regen-baseline] summary: $(cat "$SUMMARY_FILE")" >&2
