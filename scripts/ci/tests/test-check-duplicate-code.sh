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
if [ "$RC" -eq 0 ]; then
    echo "TEST FAIL: duplicated fixtures accepted (should have failed)"
    exit 1
fi
echo "PASS: duplicated case rejected with exit $RC"

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
echo "ALL TESTS PASSED"
