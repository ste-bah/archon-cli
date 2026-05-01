#!/usr/bin/env bash
# archon-init-test.sh — smoke test for scripts/archon-init.sh
#
# Does NOT test --archon-cli-repo copy (needs a real repo tree).
# Run via: bash scripts/tests/archon-init-test.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INIT_SCRIPT="$SCRIPT_DIR/../archon-init.sh"

if [ ! -f "$INIT_SCRIPT" ]; then
    echo "FAIL: archon-init.sh not found at $INIT_SCRIPT"
    exit 1
fi

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

echo "=== Test 1: basic init ==="
bash "$INIT_SCRIPT" --target "$TMPDIR"
for dir in .archon .archon/skills .archon/templates .archon/adr .archon/context .archon/agents prds tasks; do
    if [ ! -d "$TMPDIR/$dir" ]; then
        echo "FAIL: missing $dir"
        exit 1
    fi
done
echo "PASS"

echo "=== Test 2: idempotent re-run ==="
bash "$INIT_SCRIPT" --target "$TMPDIR"
echo "PASS"

echo "=== Test 3: --no-agents skips agents dir ==="
TMPDIR2=$(mktemp -d)
trap 'rm -rf "$TMPDIR" "$TMPDIR2"' EXIT
bash "$INIT_SCRIPT" --target "$TMPDIR2" --no-agents
if [ -d "$TMPDIR2/.archon/agents" ]; then
    echo "FAIL: agents dir exists but --no-agents was given"
    exit 1
fi
for dir in .archon .archon/skills .archon/templates .archon/adr .archon/context prds tasks; do
    if [ ! -d "$TMPDIR2/$dir" ]; then
        echo "FAIL: missing $dir"
        exit 1
    fi
done
echo "PASS"

echo "=== Test 4: .gitignore created with .archon entry ==="
if ! grep -q '\.archon' "$TMPDIR/.gitignore"; then
    echo "FAIL: .gitignore missing .archon entry"
    exit 1
fi
echo "PASS"

echo "=== Test 5: --help exits 0 ==="
bash "$INIT_SCRIPT" --help > /dev/null
echo "PASS"

echo "=== Test 6: invalid flag exits 1 ==="
if bash "$INIT_SCRIPT" --nonexistent 2>/dev/null; then
    echo "FAIL: expected non-zero exit for unknown flag"
    exit 1
fi
echo "PASS"

echo ""
echo "All tests passed."
