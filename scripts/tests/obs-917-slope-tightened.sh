#!/usr/bin/env bash
# Gate-1 test for TASK-AGS-OBS-917-TIGHTEN.
# Asserts the SLOPE_THRESHOLD_BYTES_PER_ITER constant in the OBS-917
# memory-slope regression test has been tightened from 1024.0 to 256.0 or
# lower (i.e., at least 4x stricter).
#
# Exits 0 on GREEN (threshold tightened), non-zero on RED.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TEST_FILE="$REPO_ROOT/crates/archon-observability/tests/obs_917_memory_slope.rs"

if [[ ! -f "$TEST_FILE" ]]; then
    echo "RED: $TEST_FILE not found"
    exit 1
fi

# Extract the numeric value assigned to SLOPE_THRESHOLD_BYTES_PER_ITER.
LINE=$(grep -n "SLOPE_THRESHOLD_BYTES_PER_ITER" "$TEST_FILE" | grep -E 'const ' | head -1)
if [[ -z "$LINE" ]]; then
    echo "RED: SLOPE_THRESHOLD_BYTES_PER_ITER const declaration not found"
    exit 1
fi
echo "Found: $LINE"

VALUE=$(echo "$LINE" | sed -E 's/.*=[[:space:]]*([0-9]+(\.[0-9]+)?).*/\1/')
if [[ -z "$VALUE" ]]; then
    echo "RED: could not extract numeric value from: $LINE"
    exit 1
fi

# Allow float comparison via awk.
CMP=$(awk -v v="$VALUE" 'BEGIN { if (v+0 <= 256.0) print "PASS"; else print "FAIL" }')
if [[ "$CMP" != "PASS" ]]; then
    echo "RED: threshold $VALUE bytes/iter is > 256.0 (must be tightened)"
    exit 1
fi

echo "GREEN: SLOPE_THRESHOLD_BYTES_PER_ITER = $VALUE (≤ 256.0)"
