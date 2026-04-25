#!/usr/bin/env bash
# tui-banned-patterns-gate.selftest.sh
#
# Runs the banned-patterns gate against fixtures under
# tests/fixtures/tui-banned-patterns/{pass,fail}, with a temporary config
# that has an empty allowlist. Asserts:
#   - pass fixture -> gate exits 0
#   - fail fixture -> gate exits non-zero AND output mentions BOUNDED_CHAN
#     and INLINE_AGENT_AWAIT

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
GATE="$REPO_ROOT/scripts/tui-banned-patterns-gate.sh"
REAL_CFG="$REPO_ROOT/config/tui-banned-patterns.json"

if [ ! -x "$GATE" ]; then
  echo "FAIL: gate script not executable: $GATE" >&2
  exit 1
fi

if [ ! -r "$REAL_CFG" ]; then
  echo "FAIL: real config not readable: $REAL_CFG" >&2
  exit 1
fi

# Build a temporary config: same rules as real config, but:
#   - allowlist emptied
#   - path_glob cleared on every rule (so TUI_MCP_IMPORT etc. apply to
#     the fixture scan root regardless of its path)
TMP_CFG="$(mktemp -t tui-banned-patterns.selftest.XXXXXX.json)"
trap 'rm -f "$TMP_CFG"' EXIT

jq '{rules: [.rules[] | . + {path_glob: ""}], allowlist: []}' \
  "$REAL_CFG" > "$TMP_CFG"

PASS_DIR="$REPO_ROOT/tests/fixtures/tui-banned-patterns/pass"
FAIL_DIR="$REPO_ROOT/tests/fixtures/tui-banned-patterns/fail"

if [ ! -d "$PASS_DIR" ] || [ ! -d "$FAIL_DIR" ]; then
  echo "FAIL: fixture directories missing under $REPO_ROOT/tests/fixtures/tui-banned-patterns" >&2
  exit 1
fi

# --- Assertion 1: pass fixture --------------------------------------------
set +e
PASS_OUT="$(
  GATE_ROOT="$REPO_ROOT" \
  BANNED_PATTERNS_JSON="$TMP_CFG" \
  SCAN_ROOTS="tests/fixtures/tui-banned-patterns/pass" \
  bash "$GATE" 2>&1
)"
PASS_RC=$?
set -e

if [ "$PASS_RC" -ne 0 ]; then
  echo "FAIL: gate returned $PASS_RC on pass fixture" >&2
  echo "$PASS_OUT" >&2
  exit 1
fi

# --- Assertion 2: fail fixture --------------------------------------------
set +e
FAIL_OUT="$(
  GATE_ROOT="$REPO_ROOT" \
  BANNED_PATTERNS_JSON="$TMP_CFG" \
  SCAN_ROOTS="tests/fixtures/tui-banned-patterns/fail" \
  bash "$GATE" 2>&1
)"
FAIL_RC=$?
set -e

if [ "$FAIL_RC" -eq 0 ]; then
  echo "FAIL: gate exited 0 on fail fixture (expected non-zero)" >&2
  echo "$FAIL_OUT" >&2
  exit 1
fi

if ! printf '%s\n' "$FAIL_OUT" | grep -q 'BOUNDED_CHAN'; then
  echo "FAIL: gate output on fail fixture did not mention BOUNDED_CHAN" >&2
  echo "$FAIL_OUT" >&2
  exit 1
fi

if ! printf '%s\n' "$FAIL_OUT" | grep -q 'INLINE_AGENT_AWAIT'; then
  echo "FAIL: gate output on fail fixture did not mention INLINE_AGENT_AWAIT" >&2
  echo "$FAIL_OUT" >&2
  exit 1
fi

echo "OK: selftest passed"
exit 0
