#!/usr/bin/env bash
# Gate-1 structural check for TASK-P1-2-PROVIDER-COUNT (#187).
# Asserts an integration test exists that verifies the native provider
# count is >= 9.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

FAIL=0

chk() {
    local name=$1 file=$2 pattern=$3
    if grep -qE "$pattern" "$REPO_ROOT/$file" 2>/dev/null; then
        echo "OK: $name"
    else
        echo "RED: $name — '$pattern' not found in $file"
        FAIL=1
    fi
}

TEST_FILE="crates/archon-llm/tests/native_provider_count.rs"

if [[ ! -f "$REPO_ROOT/$TEST_FILE" ]]; then
    echo "RED: $TEST_FILE missing"
    FAIL=1
else
    echo "OK: $TEST_FILE present"
fi

if [[ -f "$REPO_ROOT/$TEST_FILE" ]]; then
    chk "calls count_native" \
        "$TEST_FILE" \
        "count_native"

    chk "asserts count >= 9 (or exact match)" \
        "$TEST_FILE" \
        ">= 9|>=9|== 9"

    chk "enumerates provider names for diagnostic" \
        "$TEST_FILE" \
        "list_native|NATIVE_REGISTRY"
fi

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: P1.2 native provider count verification present"
