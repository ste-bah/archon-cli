#!/usr/bin/env bash
# Gate-1 test for OBS-split (metrics.rs 520 -> sub-modules).
# Asserts:
#   1. crates/archon-observability/src/metrics.rs no longer exists as a file
#   2. crates/archon-observability/src/metrics/ directory exists
#   3. Every .rs file under src/metrics/ is < 500 lines
#   4. metrics.rs is NOT in scripts/check-file-sizes.allowlist
#   5. scripts/check-file-sizes.sh exits 0 on the tree
#
# Exits 0 on GREEN (split complete and clean), non-zero on RED.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
OBS_SRC="$REPO_ROOT/crates/archon-observability/src"
ALLOWLIST="$REPO_ROOT/scripts/check-file-sizes.allowlist"

FAIL=0

# Check 1: metrics.rs single-file gone
if [[ -f "$OBS_SRC/metrics.rs" ]]; then
    echo "RED: $OBS_SRC/metrics.rs still exists as a single file; expected split into metrics/ dir"
    FAIL=1
fi

# Check 2: metrics/ dir exists
if [[ ! -d "$OBS_SRC/metrics" ]]; then
    echo "RED: $OBS_SRC/metrics/ directory does not exist"
    FAIL=1
else
    count=$(find "$OBS_SRC/metrics" -maxdepth 1 -name '*.rs' -type f | wc -l)
    if [[ "$count" -lt 2 ]]; then
        echo "RED: metrics/ has only $count .rs file(s); expected >= 2 for a meaningful split"
        FAIL=1
    fi
fi

# Check 3: every new metrics/*.rs < 500 lines
if [[ -d "$OBS_SRC/metrics" ]]; then
    while IFS= read -r -d '' f; do
        lines=$(wc -l <"$f")
        if [[ "$lines" -ge 500 ]]; then
            echo "RED: $f has $lines lines (must be < 500)"
            FAIL=1
        else
            echo "OK: $f = $lines lines"
        fi
    done < <(find "$OBS_SRC/metrics" -name '*.rs' -type f -print0)
fi

# Check 4: metrics.rs gone from allowlist
if grep -q "archon-observability/src/metrics.rs" "$ALLOWLIST"; then
    echo "RED: 'archon-observability/src/metrics.rs' still in $ALLOWLIST"
    FAIL=1
fi

# Check 5: file-sizes check exits 0 on tree
if ! bash "$REPO_ROOT/scripts/check-file-sizes.sh" >/tmp/check-file-sizes.out 2>&1; then
    echo "RED: scripts/check-file-sizes.sh exited non-zero:"
    cat /tmp/check-file-sizes.out
    FAIL=1
fi

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "FAIL"
    exit 1
fi

echo ""
echo "GREEN: metrics.rs split, all sub-files <500 lines, allowlist clean, file-size gate passes"
