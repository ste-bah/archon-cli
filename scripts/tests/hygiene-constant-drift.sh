#!/usr/bin/env bash
# Gate-1 structural check for TASK-HYGIENE-CONSTANT-DRIFT (#199).
# Asserts:
#   1. EXPECTED_PRIMARY_COUNT (dispatcher.rs) = 49
#   2. EXPECTED_COMMAND_COUNT (registry.rs)  = 49
#   3. Test `registry_primary_count_matches_expected_forty` renamed
#      (no hardcoded number in identifier)
#   4. No stale literal "= 40" on lines containing EXPECTED_*_COUNT
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

chk_no() {
    local name=$1 file=$2 pattern=$3
    if grep -qE "$pattern" "$REPO_ROOT/$file" 2>/dev/null; then
        echo "RED: $name — '$pattern' still present in $file"
        FAIL=1
    else
        echo "OK: $name"
    fi
}

chk "dispatcher.rs EXPECTED_PRIMARY_COUNT = 49" \
    "src/command/dispatcher.rs" \
    "const EXPECTED_PRIMARY_COUNT: usize = 49"

chk "registry.rs EXPECTED_COMMAND_COUNT = 49" \
    "src/command/registry.rs" \
    "const EXPECTED_COMMAND_COUNT: usize = 49"

chk_no "dispatcher.rs no stale = 40 on EXPECTED_PRIMARY_COUNT line" \
    "src/command/dispatcher.rs" \
    "const EXPECTED_PRIMARY_COUNT: usize = 40"

chk_no "registry.rs no stale = 40 on EXPECTED_COMMAND_COUNT line" \
    "src/command/registry.rs" \
    "const EXPECTED_COMMAND_COUNT: usize = 40"

chk_no "test identifier no longer has hardcoded 'forty'" \
    "src/command/dispatcher.rs" \
    "fn registry_primary_count_matches_expected_forty"

chk "renamed test identifier present" \
    "src/command/dispatcher.rs" \
    "fn registry_primary_count_matches_expected_count"

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: EXPECTED_*_COUNT bumped to 49, test identifier renamed"
