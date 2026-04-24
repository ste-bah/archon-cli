#!/usr/bin/env bash
# Gate-1 structural check for TASK-P0-B.4 (#175) Skill tool.
# Verifies the LLM-callable Skill tool is present + registered:
#   1. crates/archon-core/src/skills/skill_tool.rs exists
#   2. SkillTool struct + impl Tool for SkillTool
#   3. skills/mod.rs exports skill_tool module
#   4. dispatch.rs registers SkillTool in create_default_registry
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

if [[ ! -f "$REPO_ROOT/crates/archon-core/src/skills/skill_tool.rs" ]]; then
    echo "RED: crates/archon-core/src/skills/skill_tool.rs does not exist"
    FAIL=1
else
    echo "OK: skill_tool.rs present"
fi

chk "SkillTool struct" \
    "crates/archon-core/src/skills/skill_tool.rs" \
    "pub struct SkillTool"

chk "impl Tool for SkillTool" \
    "crates/archon-core/src/skills/skill_tool.rs" \
    "impl .*Tool for SkillTool"

chk "skills/mod.rs exports skill_tool" \
    "crates/archon-core/src/skills/mod.rs" \
    "pub mod skill_tool"

chk "dispatch.rs registers SkillTool" \
    "crates/archon-core/src/dispatch.rs" \
    "skill_tool::SkillTool"

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: P0-B.4 Skill tool surfaces present"
