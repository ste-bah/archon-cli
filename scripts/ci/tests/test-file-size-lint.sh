#!/usr/bin/env bash
# Self-test for check-tui-file-sizes.sh
# - 501-line file under TUI_SRC_ROOT must cause exit 1 and "FAIL" stdout
# - 499-line file under TUI_SRC_ROOT must cause exit 0
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LINTER="${SCRIPT_DIR}/../check-tui-file-sizes.sh"

if [[ ! -x "$LINTER" ]]; then
    echo "FAIL: linter not executable at $LINTER"
    exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/sub"

# --- Case 1: 501-line file must fail ---
printf 'fn x() {}\n%.0s' $(seq 1 501) > "$TMP/sub/big.rs"
# Ensure exactly 501 lines
actual=$(wc -l <"$TMP/sub/big.rs")
if [ "$actual" -ne 501 ]; then
    echo "TEST SETUP BUG: expected 501 lines, got $actual"
    exit 1
fi

set +e
OUT=$(TUI_SRC_ROOT="$TMP" bash "$LINTER" 2>&1)
RC=$?
set -e

if [ "$RC" -eq 0 ]; then
    echo "TEST FAIL: linter accepted 501-line file (should have failed)"
    echo "Output: $OUT"
    exit 1
fi

if ! echo "$OUT" | grep -q "FAIL"; then
    echo "TEST FAIL: linter exit was $RC but stdout lacks 'FAIL'"
    echo "Output: $OUT"
    exit 1
fi

echo "PASS: 501-line case rejected with exit $RC and FAIL message"

# --- Case 2: 499-line file must pass ---
rm -f "$TMP/sub/big.rs"
printf 'fn x() {}\n%.0s' $(seq 1 499) > "$TMP/sub/small.rs"
actual=$(wc -l <"$TMP/sub/small.rs")
if [ "$actual" -ne 499 ]; then
    echo "TEST SETUP BUG: expected 499 lines, got $actual"
    exit 1
fi

set +e
OUT=$(TUI_SRC_ROOT="$TMP" bash "$LINTER" 2>&1)
RC=$?
set -e

if [ "$RC" -ne 0 ]; then
    echo "TEST FAIL: linter rejected 499-line file (should have passed)"
    echo "Output: $OUT"
    exit 1
fi

echo "PASS: 499-line case accepted with exit 0"
echo "ALL TESTS PASSED"
