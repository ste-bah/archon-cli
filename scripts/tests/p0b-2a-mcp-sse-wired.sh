#!/usr/bin/env bash
# Gate-1 structural check for TASK-P0-B-2A-MCP-SSE-PRIMITIVES (#181).
#
# Scope: verifies the SSE framing primitives (parser + stream reader) are
# present. Full wire-ready MCP SSE transport (POST channel + rmcp adapter
# + lifecycle dispatch) is descoped to TASK-P0-B-2A-WIRE (#197).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

FAIL=0

chk() {
    local name=$1 file=$2 pattern=$3
    if grep -qE "$pattern" "$REPO_ROOT/$file" 2>/dev/null; then
        echo "OK: $name"
    else
        echo "RED: $name — pattern '$pattern' not found in $file"
        FAIL=1
    fi
}

if [[ ! -f "$REPO_ROOT/crates/archon-mcp/src/sse_transport.rs" ]]; then
    echo "RED: crates/archon-mcp/src/sse_transport.rs does not exist"
    FAIL=1
else
    echo "OK: sse_transport.rs present"
fi

chk "create_sse_transport fn" \
    "crates/archon-mcp/src/sse_transport.rs" \
    "pub fn create_sse_transport"

chk "archon-mcp lib.rs declares pub mod sse_transport" \
    "crates/archon-mcp/src/lib.rs" \
    "pub mod sse_transport"

chk "rmcp features include client-side-sse" \
    "crates/archon-mcp/Cargo.toml" \
    "\"client-side-sse\""

chk "sse_transport has end-to-end mock test" \
    "crates/archon-mcp/src/sse_transport.rs" \
    "axum::Router|axum::serve|TestServer|tokio::net::TcpListener"

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: P0-B-2A MCP SSE primitives (parser + stream reader) present"
