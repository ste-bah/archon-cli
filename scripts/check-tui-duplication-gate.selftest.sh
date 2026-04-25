#!/usr/bin/env bash
# check-tui-duplication-gate.selftest.sh
# Injection test: proves check-tui-duplication.sh exits 1 when duplication > 5%.

set -euo pipefail

REPO=$(git rev-parse --show-toplevel)
GATE="$REPO/scripts/check-tui-duplication.sh"
FAIL_FIXTURE="$REPO/tests/fixtures/tui-duplication-gate/fail"

# --- Injection test: fail fixture should exit non-zero -----------------
# Create a temp dir that looks like crates/archon-tui/src and run the gate against it.
#
# The fail fixture (tests/fixtures/tui-duplication-gate/fail/lib.rs) contains
# two 30-line identical blocks — well above the 5% / 20-line threshold.
# The gate must exit 1 when scanning this fixture.

TMPDIR=$(mktemp -d)
cleanup() { rm -rf "$TMPDIR"; }
trap cleanup EXIT

mkdir -p "$TMPDIR/src"
cp "$FAIL_FIXTURE/lib.rs" "$TMPDIR/src/lib.rs"

# Run the gate with TUI_SRC pointing at our temp fixture.
# The script's TUI_SRC="${TUI_SRC:-...}" allows override without sed.
set +e
fail_output=$(TUI_SRC="$TMPDIR/src" bash "$GATE" 2>&1)
fail_rc=$?
set -e

if [[ "$fail_rc" -eq 0 ]]; then
    echo "SELFTEST FAIL: fail fixture (30-line identical blocks) did NOT exit 1" >&2
    echo "$fail_output" >&2
    exit 1
fi

if ! grep -q "EXCEEDED" <<<"$fail_output" && ! grep -q "duplication exceeds" <<<"$fail_output" && ! grep -q "too many duplicates" <<<"$fail_output"; then
    echo "SELFTEST FAIL: fail fixture output missing threshold-exceeded marker" >&2
    echo "$fail_output" >&2
    exit 1
fi

echo "SELFTEST OK: fail fixture (30-line identical blocks) exited 1 — gate fires correctly"
exit 0
