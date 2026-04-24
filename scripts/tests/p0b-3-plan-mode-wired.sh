#!/usr/bin/env bash
# Gate-1 structural check for TASK-P0-B.3 (#174) Plan Mode state + tool interception.
# Verifies 5 surfaces:
#   A. plan_file helper module with plan_path / read / write / open_in_editor
#   B. From<&PermissionMode> for AgentMode bridge (D3)
#   C. PlanHandler parses "open" sub-arg + reads plan file when mode=plan
#   D. Dispatch appends intercepted tool call to .archon/plan.md
#   E. No TUI-626 scope-boundary deferral marker remains in plan.rs
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

FAIL=0

chk() {
    local name=$1 file=$2 pattern=$3
    if grep -qE "$pattern" "$REPO_ROOT/$file" 2>/dev/null; then
        echo "OK: $name"
    else
        echo "RED: $name — pattern '$pattern' not found in $file"
        FAIL=1
    fi
}

chk_no() {
    local name=$1 file=$2 pattern=$3
    if grep -qE "$pattern" "$REPO_ROOT/$file" 2>/dev/null; then
        echo "RED: $name — '$pattern' still present in $file"
        FAIL=1
    else
        echo "OK: $name"
    fi
}

# A. plan_file helper module
if [[ ! -f "$REPO_ROOT/src/command/plan_file.rs" ]]; then
    echo "RED: src/command/plan_file.rs does not exist"
    FAIL=1
else
    echo "OK: plan_file.rs present"
fi

chk "plan_path helper" \
    "src/command/plan_file.rs" \
    "pub fn plan_path"

chk "read_plan_file helper" \
    "src/command/plan_file.rs" \
    "pub fn read_plan_file"

chk "append_plan_entry helper" \
    "src/command/plan_file.rs" \
    "pub fn append_plan_entry"

chk "open_plan_in_editor helper" \
    "src/command/plan_file.rs" \
    "pub fn open_plan_in_editor"

# B. D3 bridge
chk "From<&PermissionMode> for AgentMode" \
    "crates/archon-tools/src/plan_mode.rs" \
    "impl From<&archon_permissions::PermissionMode> for AgentMode|impl From<.*PermissionMode.*> for AgentMode"

# C. PlanHandler handles "open" sub-arg + reads plan when mode=plan
chk "PlanHandler references plan_file" \
    "src/command/plan.rs" \
    "plan_file::|crate::command::plan_file"

chk "PlanHandler handles 'open' arg" \
    "src/command/plan.rs" \
    "\"open\""

# D. Dispatch interception appends to plan.md
chk "Dispatch interception appends to plan file" \
    "crates/archon-core/src/dispatch.rs" \
    "append_plan_entry|plan_file::"

# E. No stale TUI-626 deferral marker in plan.rs scope-boundary section
chk_no "plan.rs drops TUI-626 scope-boundary deferral language" \
    "src/command/plan.rs" \
    "deferred to P0-B\\.3"

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: P0-B.3 Plan Mode surfaces present"
