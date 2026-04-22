#!/usr/bin/env bash
# Self-test for check-cycles.sh
# - Cyclic DOT (A->B, B->A) must produce exit 1 + cycle-member output
# - Acyclic DOT (A->B, B->C) must produce exit 0
# Uses DEPGRAPH_OVERRIDE so cargo-depgraph is not required at test time.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECKER="${SCRIPT_DIR}/../check-cycles.sh"

if [[ ! -x "$CHECKER" ]]; then
    echo "FAIL: check-cycles.sh not executable at $CHECKER"
    exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# --- Case 1: cyclic A->B, B->A must fail ---
cat > "$TMP/cyclic.dot" <<'EOF'
digraph dep {
    "A" -> "B";
    "B" -> "A";
    "B" -> "C";
}
EOF

set +e
OUT=$(DEPGRAPH_OVERRIDE="$TMP/cyclic.dot" bash "$CHECKER" 2>&1)
RC=$?
set -e

if [ "$RC" -eq 0 ]; then
    echo "TEST FAIL: cyclic DOT accepted (should have failed)"
    echo "Output: $OUT"
    exit 1
fi

# Output should name the cycle members (A and B)
if ! echo "$OUT" | grep -qE 'A|B'; then
    echo "TEST FAIL: cyclic DOT exit was $RC but stdout does not name cycle members"
    echo "Output: $OUT"
    exit 1
fi

echo "PASS: cyclic case rejected with exit $RC and cycle members named"

# --- Case 2: acyclic A->B->C must pass ---
cat > "$TMP/acyclic.dot" <<'EOF'
digraph dep {
    "A" -> "B";
    "B" -> "C";
}
EOF

set +e
OUT=$(DEPGRAPH_OVERRIDE="$TMP/acyclic.dot" bash "$CHECKER" 2>&1)
RC=$?
set -e

if [ "$RC" -ne 0 ]; then
    echo "TEST FAIL: acyclic DOT rejected (should have passed)"
    echo "Output: $OUT"
    exit 1
fi

if ! echo "$OUT" | grep -q "OK"; then
    echo "TEST FAIL: acyclic exit 0 but no OK message"
    echo "Output: $OUT"
    exit 1
fi

echo "PASS: acyclic case accepted with exit 0 and OK message"
echo "ALL TESTS PASSED"
