#!/usr/bin/env bash
# check-preserve-invariants.sh
# Fraud-detection CI script that greps the workspace for accidental
# reintroduction of patterns that violate REQ-FOR-PRESERVE-D5/D8.
#
# Usage:
#   ./check-preserve-invariants.sh
#   ./check-preserve-invariants.sh --self-test

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BASE_DIR="${BASE_DIR:-$SCRIPT_DIR/..}"

diag() {
    echo "Reference: TECH-AGS-PRESERVE, prds/archon-cli-forensic-analysis.md line 707"
}

run_checks() {
    local FAILED=0

    # --- Check 1: mcp__memorygraph__ leak ---
    # Exclude tests/ (legitimate verification), archon-mcp/ (bridge registration),
    # and comment-only lines.
    local MATCHES=""
    while IFS= read -r line; do
        # Skip lines from test directories
        [[ "$line" =~ /tests/ ]] && continue
        # Skip bridge registration files
        [[ "$line" =~ archon-mcp/src/tool_bridge\.rs ]] && continue
        [[ "$line" =~ command/mcp\.rs ]] && continue
        # Skip pure doc comments (extract content after filename:line_num:)
        local text="${line#*:}"
        text="${text#*:}"
        text="${text#"${text%%[![:space:]]*}"}"  # ltrim
        [[ "$text" =~ ^(//|///|//!) ]] && continue
        MATCHES+="$line"$'\n'
    done < <(grep -rn "mcp__memorygraph__" "$BASE_DIR/crates" "$BASE_DIR/src" 2>/dev/null || true)

    if [[ -n "$MATCHES" ]]; then
        echo "FAIL: mcp__memorygraph__ leak detected in source files."
        echo "Invariant: REQ-FOR-PRESERVE-D5/D8 -- MemoryGraph MCP strings belong in outer-Archon docs/CLAUDE.md only."
        diag
        echo "Offending matches:"
        echo "$MATCHES"
        FAILED=1
    fi

    # --- Check 2: ad-hoc file-based memory write ---
    MATCHES=""
    local search_paths=()
    for p in "$BASE_DIR"/crates/*/src/; do
        [[ -d "$p" ]] && search_paths+=("$p")
    done
    if [[ -d "$BASE_DIR/src" ]]; then
        search_paths+=("$BASE_DIR/src")
    fi
    if [[ ${#search_paths[@]} -gt 0 ]]; then
        MATCHES=$(grep -rnE 'std::fs::write\([^)]*memory' "${search_paths[@]}" 2>/dev/null || true)
    fi
    if [[ -n "$MATCHES" ]]; then
        echo "FAIL: Ad-hoc file-based memory write detected."
        echo "Invariant: REQ-FOR-PRESERVE-D5/D8 -- NEVER use file-based auto-memory."
        diag
        echo "Offending matches:"
        echo "$MATCHES"
        FAILED=1
    fi

    # --- Check 3: AUTO_BACKGROUND_MS constant ---
    if ! grep -q "pub const AUTO_BACKGROUND_MS: u64 = 120_000" "$BASE_DIR/crates/archon-core/src/subagent.rs" 2>/dev/null; then
        echo "MISSING: AUTO_BACKGROUND_MS constant"
        echo "Invariant: REQ-FOR-PRESERVE-D5/D8 -- AUTO_BACKGROUND_MS must be present."
        diag
        FAILED=1
    fi

    # --- Check 4: AGT-025 race structure in agent_tool.rs ---
    local AGENT_TOOL="$BASE_DIR/crates/archon-tools/src/agent_tool.rs"
    if [[ ! -f "$AGENT_TOOL" ]]; then
        echo "MISSING: $AGENT_TOOL"
        diag
        FAILED=1
    else
        if ! grep -q "tokio::select!" "$AGENT_TOOL"; then
            echo "MISSING: tokio::select! anchor in agent_tool.rs"
            diag
            FAILED=1
        fi
        if ! grep -q "join_handle" "$AGENT_TOOL"; then
            echo "MISSING: join_handle anchor in agent_tool.rs"
            diag
            FAILED=1
        fi
        if ! grep -q "tokio::time::sleep(Duration::from_millis(auto_bg_ms))" "$AGENT_TOOL"; then
            echo "MISSING: tokio::time::sleep(Duration::from_millis(auto_bg_ms)) anchor in agent_tool.rs"
            diag
            FAILED=1
        fi
    fi

    # --- Check 5: save_agent_memory signature ---
    local MEM_FILE="$BASE_DIR/crates/archon-core/src/agents/memory.rs"
    if [[ ! -f "$MEM_FILE" ]]; then
        echo "MISSING: $MEM_FILE"
        diag
        FAILED=1
    else
        if ! grep -q "pub fn save_agent_memory" "$MEM_FILE"; then
            echo "MISSING: pub fn save_agent_memory in memory.rs"
            diag
            FAILED=1
        fi
        if ! grep -q "memory: &dyn MemoryTrait" "$MEM_FILE"; then
            echo "MISSING: memory: &dyn MemoryTrait argument in save_agent_memory"
            diag
            FAILED=1
        fi
        if ! grep -q "memory_scope: Option<&AgentMemoryScope>" "$MEM_FILE"; then
            echo "MISSING: memory_scope: Option<&AgentMemoryScope> argument in save_agent_memory"
            diag
            FAILED=1
        fi
    fi

    return "$FAILED"
}

if [[ "${1:-}" == "--self-test" ]]; then
    TMP=$(mktemp -d)
    trap 'rm -rf "$TMP"' EXIT

    mkdir -p "$TMP/crates/archon-core/src/agents"
    mkdir -p "$TMP/crates/archon-tools/src"
    mkdir -p "$TMP/src"

    # Synthetic violation for check 1 (must be actual code, not a comment)
    echo "fn leak() { mcp__memorygraph__self_test(); }" > "$TMP/crates/archon-core/src/lib.rs"

    # Stubs so the other checks pass
    echo "pub const AUTO_BACKGROUND_MS: u64 = 120_000;" > "$TMP/crates/archon-core/src/subagent.rs"
    cat > "$TMP/crates/archon-tools/src/agent_tool.rs" <<'EOF'
tokio::select! {
    _ = join_handle => {},
    _ = tokio::time::sleep(Duration::from_millis(auto_bg_ms)) => {},
}
EOF
    cat > "$TMP/crates/archon-core/src/agents/memory.rs" <<'EOF'
pub fn save_agent_memory(memory: &dyn MemoryTrait, memory_scope: Option<&AgentMemoryScope>) {}
EOF

    if BASE_DIR="$TMP" run_checks; then
        echo "SELF-TEST FAIL: script did not detect synthetic violation"
        exit 1
    fi
    echo "SELF-TEST PASS"
    exit 0
fi

if ! run_checks; then
    exit 1
fi

echo "All REQ-FOR-PRESERVE-D5/D8 invariants passed."
exit 0
