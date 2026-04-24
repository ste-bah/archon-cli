#!/usr/bin/env bash
# Gate-1 structural check for TASK-TUI-620-FOLLOWUP.
# Verifies 4 deferred surfaces now have wiring in the tree:
#   1. MessageSelector::render method defined
#   2. draw_message_selector fn in render/body.rs
#   3. render/mod.rs draw() invokes draw_message_selector
#   4. event_loop/input.rs has message_selector priority branch
#   5. src/command/rewind.rs RealMessageLoader no longer carries the stub marker
#   6. src/session.rs has __truncate_session__ consumer
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

chk "MessageSelector::render method" \
    "crates/archon-tui/src/screens/message_selector.rs" \
    "pub fn render\\("

chk "draw_message_selector in render/body.rs" \
    "crates/archon-tui/src/render/body.rs" \
    "pub fn draw_message_selector"

chk "render/mod.rs invokes draw_message_selector" \
    "crates/archon-tui/src/render/mod.rs" \
    "draw_message_selector"

chk "event_loop/input.rs priority branch for message_selector" \
    "crates/archon-tui/src/event_loop/input.rs" \
    "app\\.message_selector\\.is_some\\(\\)"

chk_no "message_selector.rs retains no TODO(TUI-620-followup)" \
    "crates/archon-tui/src/screens/message_selector.rs" \
    "TODO\\(TUI-620-followup\\)"

chk_no "RealMessageLoader retains no empty-Vec stub marker" \
    "src/command/rewind.rs" \
    "TODO\\(TUI-620-followup\\): wire to archon_session::storage::SessionStore"

chk "session.rs has __truncate_session__ consumer" \
    "src/session.rs" \
    "__truncate_session__"

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: TUI-620-followup surfaces present"
