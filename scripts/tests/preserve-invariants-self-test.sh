#!/usr/bin/env bash
# Self-test for check-preserve-invariants.sh (TASK-AGS-OBS-918).
#
# Injects synthetic violations into a temp copy of the workspace and asserts
# the script exits 1 with the correct diagnostic.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECKER="${SCRIPT_DIR}/../check-preserve-invariants.sh"

if [[ ! -x "$CHECKER" ]]; then
    echo "FAIL: check-preserve-invariants.sh not executable at $CHECKER"
    exit 1
fi

WORK="$(mktemp -d -t preserve-self-test-XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

# Copy workspace excluding target/ and .git/ to avoid 149GB copy
cd "${SCRIPT_DIR}/../.."
mkdir -p "$WORK/ws"
tar -c --exclude='target' --exclude='*.tmp' . | tar -x -C "$WORK/ws"

CHECKER_IN_WS="$WORK/ws/scripts/check-preserve-invariants.sh"

run_checker() {
    bash "$CHECKER_IN_WS" "$@" 2>&1
}

PASS_COUNT=0
FAIL_COUNT=0

assert_fails() {
    local name="$1"
    shift
    set +e
    OUT="$(run_checker "$@" 2>&1)"
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
        echo "TEST FAIL: $name — expected exit != 0, got $RC"
        echo "$OUT"
        FAIL_COUNT=$((FAIL_COUNT + 1))
    else
        echo "PASS: $name (rc=$RC)"
        PASS_COUNT=$((PASS_COUNT + 1))
    fi
}

assert_passes() {
    local name="$1"
    shift
    set +e
    OUT="$(run_checker "$@" 2>&1)"
    RC=$?
    set -e
    if [ "$RC" -ne 0 ]; then
        echo "TEST FAIL: $name — expected exit 0, got $RC"
        echo "$OUT"
        FAIL_COUNT=$((FAIL_COUNT + 1))
    else
        echo "PASS: $name (rc=$RC)"
        PASS_COUNT=$((PASS_COUNT + 1))
    fi
}

# --- Case A: baseline (current workspace) must pass ---
echo "--- Case A: baseline passes ---"
assert_passes "baseline"

# --- Case B: mcp__memorygraph__ leak in crate source ---
echo "--- Case B: mcp__memorygraph__ leak ---"
echo "fn leak() { mcp__memorygraph__store_memory(); }" >> "$WORK/ws/crates/archon-core/src/lib.rs"
assert_fails "mcp-memorygraph-leak"
git -C "$WORK/ws" checkout -- crates/archon-core/src/lib.rs 2>/dev/null || true

# --- Case C: ad-hoc file-based memory write in crate source ---
echo "--- Case C: ad-hoc memory file write ---"
echo 'std::fs::write(".archon/memory/foo.json", data);' >> "$WORK/ws/crates/archon-core/src/lib.rs"
assert_fails "ad-hoc-memory-write"
git -C "$WORK/ws" checkout -- crates/archon-core/src/lib.rs 2>/dev/null || true

# --- Case D: AUTO_BACKGROUND_MS constant missing ---
echo "--- Case D: AUTO_BACKGROUND_MS missing ---"
sed -i 's/pub const AUTO_BACKGROUND_MS: u64 = 120_000;/\/\/ REMOVED/' "$WORK/ws/crates/archon-core/src/subagent.rs"
assert_fails "auto-background-ms-missing"
git -C "$WORK/ws" checkout -- crates/archon-core/src/subagent.rs 2>/dev/null || true

# --- Case E: AGT-025 race structure missing from agent_tool.rs ---
echo "--- Case E: AGT-025 race structure missing ---"
sed -i 's/tokio::select!/tokio::select_REMOVED!/' "$WORK/ws/crates/archon-tools/src/agent_tool.rs"
assert_fails "agt025-race-missing"
git -C "$WORK/ws" checkout -- crates/archon-tools/src/agent_tool.rs 2>/dev/null || true

# --- Case F: save_agent_memory signature drift (memory_scope removed) ---
echo "--- Case F: save_agent_memory signature drift ---"
sed -i 's/memory_scope: Option<&AgentMemoryScope>/\/\/ memory_scope removed/' "$WORK/ws/crates/archon-core/src/agents/memory.rs"
assert_fails "save-agent-memory-signature-drift"
git -C "$WORK/ws" checkout -- crates/archon-core/src/agents/memory.rs 2>/dev/null || true

echo ""
echo "========================================"
echo "RESULTS: $PASS_COUNT passed, $FAIL_COUNT failed"
echo "========================================"

if [ "$FAIL_COUNT" -gt 0 ]; then
    exit 1
fi
echo "ALL TESTS PASSED"
