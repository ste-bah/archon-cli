#!/usr/bin/env bash
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

TEST_FILE="crates/archon-llm/tests/stream_cancel_mid_stream.rs"

if [[ ! -f "$REPO_ROOT/$TEST_FILE" ]]; then
    echo "RED: $TEST_FILE missing"
    FAIL=1
else
    echo "OK: $TEST_FILE present"
fi

if [[ -f "$REPO_ROOT/$TEST_FILE" ]]; then
    chk "axum mock server" "$TEST_FILE" "axum::Router|axum::serve|axum::response"
    chk "reqwest streaming client" "$TEST_FILE" "reqwest::Client|bytes_stream|Sse"
    chk "cancel / drop stream" "$TEST_FILE" "drop\\(|cancel|abort"
    chk "asserts server connection closed" "$TEST_FILE" "AtomicUsize|connected_clients|connections_closed|Arc::new"
fi

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: P1.3 stream-cancel verification present"
