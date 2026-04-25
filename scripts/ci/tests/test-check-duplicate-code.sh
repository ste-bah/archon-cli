#!/usr/bin/env bash
# Self-test for check-duplicate-code.sh
# - Duplicated fixtures (a.rs === b.rs, ~92 lines each) must cause exit 1
# - Unique fixture (single small file) must cause exit 0
# Uses JSCPD_TARGET_DIR override to point at test fixtures.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECKER="${SCRIPT_DIR}/../check-duplicate-code.sh"
FIXTURE_DUP="${SCRIPT_DIR}/fixtures/duplicated"
FIXTURE_UNIQ="${SCRIPT_DIR}/fixtures/unique"

if [[ ! -x "$CHECKER" ]]; then
    echo "FAIL: check-duplicate-code.sh not executable at $CHECKER"
    exit 1
fi

# --- Case 1: duplicated fixtures must fail ---
echo "--- Case 1: duplicated fixtures (expect exit 1) ---"
set +e
OUT=$(JSCPD_TARGET_DIR="$FIXTURE_DUP" JSCPD_REPORT_DIR="/tmp/jscpd-report-dup" bash "$CHECKER" 2>&1)
RC=$?
set -e
echo "$OUT"
if [ "$RC" -ne 1 ]; then
    echo "TEST FAIL: duplicated fixtures exit=$RC, expected 1"
    exit 1
fi
echo "PASS: duplicated case rejected with exit 1"

# --- Case 2: unique fixture must pass ---
echo "--- Case 2: unique fixture (expect exit 0) ---"
set +e
OUT=$(JSCPD_TARGET_DIR="$FIXTURE_UNIQ" JSCPD_REPORT_DIR="/tmp/jscpd-report-uniq" bash "$CHECKER" 2>&1)
RC=$?
set -e
echo "$OUT"
if [ "$RC" -ne 0 ]; then
    echo "TEST FAIL: unique fixture rejected (should have passed)"
    exit 1
fi
echo "PASS: unique case accepted with exit 0"

# --- Case 3: missing target dir must exit 2 ---
echo "--- Case 3: missing target dir (expect exit 2) ---"
set +e
OUT=$(JSCPD_TARGET_DIR="/tmp/nonexistent-jscpd-target-$$" bash "$CHECKER" 2>&1)
RC=$?
set -e
if [ "$RC" -ne 2 ]; then
    echo "TEST FAIL: missing target dir exit=$RC, expected 2"
    exit 1
fi
echo "PASS: missing target dir exit 2"

# --- Case 4: missing npx must exit 2 ---
echo "--- Case 4: missing npx (expect exit 2) ---"
set +e
OUT=$(PATH="/usr/bin:/bin" bash "$CHECKER" 2>&1)
RC=$?
set -e
if [ "$RC" -ne 2 ]; then
    echo "TEST FAIL: missing npx exit=$RC, expected 2"
    exit 1
fi
echo "PASS: missing npx exit 2"

echo "ALL TESTS PASSED"
