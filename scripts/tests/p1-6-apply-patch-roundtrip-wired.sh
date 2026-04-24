#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

FAIL=0
TEST_FILE="crates/archon-tools/tests/apply_patch_roundtrip.rs"

chk() {
    local name=$1 pattern=$2
    if grep -qE "$pattern" "$REPO_ROOT/$TEST_FILE" 2>/dev/null; then
        echo "OK: $name"
    else
        echo "RED: $name — '$pattern' not found"
        FAIL=1
    fi
}

if [[ ! -f "$REPO_ROOT/$TEST_FILE" ]]; then
    echo "RED: $TEST_FILE missing"
    FAIL=1
else
    echo "OK: $TEST_FILE present"
fi

if [[ -f "$REPO_ROOT/$TEST_FILE" ]]; then
    chk "multi-hunk patch" "@@ -.+@@.*@@ -"
    chk "apply + reverse patch" "reverse|apply.*twice|inverse"
    chk "sha256 byte-for-byte assertion" "sha2|Sha256|digest"
    chk "tempfile tempdir" "tempfile|tempdir"
fi

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: P1.6 apply_patch round-trip verification present"
