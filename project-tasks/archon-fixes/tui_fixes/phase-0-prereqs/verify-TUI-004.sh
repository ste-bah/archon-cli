#!/usr/bin/env bash
# Reference: project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/TASK-TUI-004.md
set -euo pipefail

cd /home/unixdude/Archon-projects/archon-cli-worktrees/archonfixes

CONFIG="config/tui-banned-patterns.json"
GATE="scripts/tui-banned-patterns-gate.sh"
SELFTEST="scripts/tui-banned-patterns-gate.selftest.sh"
FIX_PASS="tests/fixtures/tui-banned-patterns/pass/clean.rs"
FIX_BOUNDED="tests/fixtures/tui-banned-patterns/fail/bounded.rs"
FIX_INLINE="tests/fixtures/tui-banned-patterns/fail/inline_await.rs"

# 1. Config file exists
if [ ! -f "$CONFIG" ]; then
  echo "FAIL: $CONFIG does not exist" >&2
  exit 1
fi

# 2. Valid JSON
if ! jq -e . "$CONFIG" >/dev/null; then
  echo "FAIL: $CONFIG is not valid JSON" >&2
  exit 1
fi

# 3. Rule count == 4
RULE_COUNT="$(jq '.rules | length' "$CONFIG")"
if [ "$RULE_COUNT" != "4" ]; then
  echo "FAIL: expected 4 rules, found $RULE_COUNT" >&2
  exit 1
fi

# 4. All four rule ids present
RULE_IDS="$(jq -r '.rules[].id' "$CONFIG")"
for id in BOUNDED_CHAN SEND_AWAIT INLINE_AGENT_AWAIT TUI_MCP_IMPORT; do
  if ! printf '%s\n' "$RULE_IDS" | grep -qx "$id"; then
    echo "FAIL: rule id $id missing from $CONFIG" >&2
    exit 1
  fi
done

# 5. Gate scripts exist and are executable
for f in "$GATE" "$SELFTEST"; do
  if [ ! -f "$f" ]; then
    echo "FAIL: $f does not exist" >&2
    exit 1
  fi
  if [ ! -x "$f" ]; then
    echo "FAIL: $f is not executable" >&2
    exit 1
  fi
done

# 6. Fixture files exist
for f in "$FIX_PASS" "$FIX_BOUNDED" "$FIX_INLINE"; do
  if [ ! -f "$f" ]; then
    echo "FAIL: fixture $f does not exist" >&2
    exit 1
  fi
done

# 7. Fail fixtures contain banned strings
if ! grep -F -q 'mpsc::channel::<AgentEvent>(256)' "$FIX_BOUNDED"; then
  echo "FAIL: $FIX_BOUNDED missing 'mpsc::channel::<AgentEvent>(256)'" >&2
  exit 1
fi
if ! grep -E -q 'process_message.*\.await' "$FIX_INLINE"; then
  echo "FAIL: $FIX_INLINE missing '.process_message(...).await' pattern" >&2
  exit 1
fi

# 8. Selftest passes
if ! bash "$SELFTEST"; then
  echo "FAIL: $SELFTEST did not exit 0" >&2
  exit 1
fi

# 9. Gate passes against real repo with day-0 allowlist
if ! bash "$GATE"; then
  echo "FAIL: $GATE did not exit 0 with day-0 allowlist" >&2
  exit 1
fi

# 10. Ratchet proof: remove first allowlist entry -> gate must fail
EMPTY_CFG="/tmp/tui-banned-patterns.empty.json"
jq 'del(.allowlist[0])' "$CONFIG" > "$EMPTY_CFG"
if BANNED_PATTERNS_JSON="$EMPTY_CFG" bash "$GATE"; then
  echo "FAIL: gate passed with shrunken allowlist; ratchet not enforced" >&2
  exit 1
fi

# 11. No implementation drift in src/ or crates/archon-tui/src/
DIRTY="$(git diff --name-only -- src/ crates/archon-tui/src/ || true)"
if [ -n "$DIRTY" ]; then
  echo "FAIL: unexpected modifications in src/ or crates/archon-tui/src/:" >&2
  echo "$DIRTY" >&2
  exit 1
fi

echo "OK: verify-TUI-004 passed"
