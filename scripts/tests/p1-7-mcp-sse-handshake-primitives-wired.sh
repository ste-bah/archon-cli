#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

FAIL=0
TEST_FILE="crates/archon-mcp/tests/sse_handshake_primitives_smoke.rs"

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
    chk "uses sse_transport primitives" "sse_transport|create_sse_transport|SseFrame|connect_sse_stream"
    chk "axum SSE echo server" "axum::Router|axum::serve|axum::response"
    chk "emits JSON-RPC initialize shape" "jsonrpc|initialize"
    chk "asserts parsed payload" "assert.*jsonrpc|method.*initialize|protocolVersion"
    chk "references #197 for full handshake defer" "#197|TASK-P0-B-2A-WIRE"
fi

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: P1.7 MCP SSE handshake PRIMITIVES smoke present"
