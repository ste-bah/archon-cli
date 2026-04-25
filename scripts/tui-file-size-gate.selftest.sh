#!/usr/bin/env bash
# tui-file-size-gate.selftest.sh
# Exercises the ratchet gate against pass/fail fixtures and asserts exit codes.

set -euo pipefail

REPO=$(git rev-parse --show-toplevel)
GATE="$REPO/scripts/tui-file-size-gate.sh"
PASS_ROOT="$REPO/tests/fixtures/tui-file-size-gate/pass"
FAIL_ROOT="$REPO/tests/fixtures/tui-file-size-gate/fail"

# --- Test 1: pass fixture should exit 0 ---------------------------------
if ! GATE_ROOT="$PASS_ROOT" \
     SCAN_ROOTS="src" \
     ALLOWLIST_PATH="$PASS_ROOT/file-size-allowlist.json" \
     bash "$GATE" >/dev/null; then
    echo "SELFTEST FAIL: pass fixture did not exit 0" >&2
    exit 1
fi

# --- Test 2: fail fixture should exit non-zero --------------------------
set +e
fail_output=$(GATE_ROOT="$FAIL_ROOT" \
              SCAN_ROOTS="src" \
              ALLOWLIST_PATH="$FAIL_ROOT/file-size-allowlist.json" \
              bash "$GATE" 2>&1)
fail_rc=$?
set -e

if [[ "$fail_rc" -eq 0 ]]; then
    echo "SELFTEST FAIL: fail fixture unexpectedly exited 0" >&2
    echo "$fail_output" >&2
    exit 1
fi

if ! grep -q "FAIL:" <<<"$fail_output"; then
    echo "SELFTEST FAIL: fail fixture output missing 'FAIL:' marker" >&2
    echo "$fail_output" >&2
    exit 1
fi

if ! grep -q "src/bad.rs" <<<"$fail_output"; then
    echo "SELFTEST FAIL: fail fixture output missing 'src/bad.rs' path" >&2
    echo "$fail_output" >&2
    exit 1
fi

echo "OK: selftest passed"
exit 0
