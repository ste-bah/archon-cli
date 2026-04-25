#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

FAIL=0
TEST_FILE="crates/archon-observability/tests/redaction_patterns_smoke.rs"

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
    # Each of the 6 pattern classes asserted by name
    chk "OpenAI sk- pattern" "sk-proj-|sk-[a-zA-Z0-9]"
    chk "Anthropic sk-ant- pattern" "sk-ant-"
    chk "AWS AKIA pattern" "AKIA"
    chk "GCP service account JSON" "service_account"
    chk "JWT Bearer pattern" "Bearer|eyJhbGc"
    chk "Generic API key pattern" "generic.*key|api_key|32"
    chk "REDACTED marker check" "\\*\\*\\*REDACTED\\*\\*\\*"
    chk "no raw secret leak check" "assert!\\(!.*contains"
fi

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: P1.5 redaction patterns verification present"
