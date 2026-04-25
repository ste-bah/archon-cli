#!/usr/bin/env bash
# Gate-1 structural check for TASK-201-SEC-REDACTION-GCP.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

FAIL=0
REDACTION_RS="crates/archon-observability/src/redaction.rs"
INTEG_TEST="crates/archon-observability/tests/redaction_patterns_smoke.rs"

chk() {
    local name=$1 file=$2 pattern=$3
    if grep -qE "$pattern" "$REPO_ROOT/$file" 2>/dev/null; then
        echo "OK: $name"
    else
        echo "RED: $name — '$pattern' not found in $file"
        FAIL=1
    fi
}

chk_no() {
    local name=$1 file=$2 pattern=$3
    if grep -qE "$pattern" "$REPO_ROOT/$file" 2>/dev/null; then
        echo "RED: $name — '$pattern' still present in $file"
        FAIL=1
    else
        echo "OK: $name"
    fi
}

# Production regex additions
chk "service_account JSON marker in REDACTION_RE" \
    "$REDACTION_RS" \
    "service_account|service[_-]account"

chk "PEM private key pattern in REDACTION_RE" \
    "$REDACTION_RS" \
    "BEGIN.*PRIVATE KEY|PRIVATE KEY"

chk "credentials field name in REDACTION_RE" \
    "$REDACTION_RS" \
    "\\| credentials"

# New unit tests
chk "test redacts_gcp_service_account_json_blob" \
    "$REDACTION_RS" \
    "fn redacts_gcp_service_account_json"

chk "test redacts_standalone_pem_private_key" \
    "$REDACTION_RS" \
    "fn redacts_standalone_pem_private_key|fn redacts_pem_private_key"

chk "test credentials field name redacted" \
    "$REDACTION_RS" \
    "fn redacts_credentials_field|fn credentials_field"

chk "ReDoS guard for multi-line GCP patterns" \
    "$REDACTION_RS" \
    "fn .*gcp_no_catastrophic_backtracking|fn .*pem_no_catastrophic"

# Integration test: un-ignore the GCP test + make strict
chk_no "integration GCP test is no longer #[ignore]" \
    "$INTEG_TEST" \
    "#\\[ignore.*#201|#\\[ignore.*SEC-REDACTION-GCP"

chk "integration GCP test exists as strict check" \
    "$INTEG_TEST" \
    "fn gcp_service_account_json_scrubbed"

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: #201 SEC-REDACTION-GCP surfaces present"
