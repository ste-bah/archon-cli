#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

FAIL=0
TEST_FILE="crates/archon-observability/tests/metrics_endpoint_e2e.rs"

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
    chk "serve_metrics_on spawn" "serve_metrics_on"
    chk "reqwest or TcpStream client" "reqwest|TcpStream|http_body"
    chk "asserts # HELP" "# HELP"
    chk "asserts # TYPE" "# TYPE"
    chk "emits channel event before scrape" "record_sent|record_drained|record_latency_ms"
fi

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: P1.4 /metrics endpoint verification present"
