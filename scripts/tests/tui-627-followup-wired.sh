#!/usr/bin/env bash
# Gate-1 structural check for TASK-TUI-627-FOLLOWUP.
# Verifies 4 deferred surfaces for /skills SkillsMenu overlay:
#   1. SkillsMenu::render method defined
#   2. draw_skills_menu fn in render/body.rs
#   3. render/mod.rs draw() invokes draw_skills_menu
#   4. event_loop/input.rs has skills_menu priority branch
#   5. skills_menu.rs no longer carries the TODO(TUI-627-followup) markers
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

chk "SkillsMenu::render method" \
    "crates/archon-tui/src/screens/skills_menu.rs" \
    "pub fn render\\("

chk "draw_skills_menu in render/body.rs" \
    "crates/archon-tui/src/render/body.rs" \
    "pub fn draw_skills_menu"

chk "render/mod.rs invokes draw_skills_menu" \
    "crates/archon-tui/src/render/mod.rs" \
    "draw_skills_menu"

chk "event_loop/input.rs priority branch for skills_menu" \
    "crates/archon-tui/src/event_loop/input.rs" \
    "app\\.skills_menu\\.is_some\\(\\)"

chk_no "skills_menu.rs retains no TODO(TUI-627-followup)" \
    "crates/archon-tui/src/screens/skills_menu.rs" \
    "TODO\\(TUI-627-followup\\)"

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: TUI-627-followup surfaces present"
