#!/usr/bin/env bash
# Gate-1 structural check for TASK-HYGIENE-INPUT-RS-SPLIT (#198).
# Pure refactor: crates/archon-tui/src/input.rs (527 lines) split into
# input/ directory module. Assertions:
#   1. Original single-file input.rs no longer present as file
#   2. input/ directory exists
#   3. Every .rs under input/ < 500 lines
#   4. Public surface preserved (InputHandler, KeyResult, handle_key)
#   5. scripts/check-file-sizes.sh reports zero archon-tui offenders
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
INPUT_DIR="$REPO_ROOT/crates/archon-tui/src/input"

FAIL=0

if [[ -f "$REPO_ROOT/crates/archon-tui/src/input.rs" ]]; then
    echo "RED: crates/archon-tui/src/input.rs still exists as single file"
    FAIL=1
else
    echo "OK: single-file input.rs absent"
fi

if [[ ! -d "$INPUT_DIR" ]]; then
    echo "RED: crates/archon-tui/src/input/ directory missing"
    FAIL=1
else
    count=$(find "$INPUT_DIR" -maxdepth 1 -name '*.rs' -type f | wc -l)
    if [[ "$count" -lt 2 ]]; then
        echo "RED: input/ has only $count .rs file(s); need >=2"
        FAIL=1
    else
        echo "OK: input/ has $count .rs files"
    fi
fi

if [[ -d "$INPUT_DIR" ]]; then
    while IFS= read -r -d '' f; do
        lines=$(wc -l <"$f")
        if [[ "$lines" -ge 500 ]]; then
            echo "RED: $f has $lines lines (>=500)"
            FAIL=1
        else
            echo "OK: $(basename "$f") = $lines lines"
        fi
    done < <(find "$INPUT_DIR" -name '*.rs' -type f -print0)
fi

# Public surface preserved: grep across the dir for key items.
chk_any() {
    local name=$1 pattern=$2
    if grep -rqE "$pattern" "$INPUT_DIR" 2>/dev/null; then
        echo "OK: $name"
    else
        echo "RED: $name — pattern '$pattern' not found under input/"
        FAIL=1
    fi
}

chk_any "InputHandler struct" "pub struct InputHandler"
chk_any "KeyResult enum" "pub enum KeyResult"
chk_any "handle_key fn" "pub fn handle_key"

# Overall file-size gate must be clean for archon-tui.
OFFENDERS=$(bash "$REPO_ROOT/scripts/check-file-sizes.sh" 2>&1 | grep -v allowlisted | grep -E "archon-tui" || true)
if [[ -n "$OFFENDERS" ]]; then
    echo "RED: archon-tui file-size offenders remain:"
    echo "$OFFENDERS"
    FAIL=1
else
    echo "OK: archon-tui has zero file-size offenders"
fi

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: input.rs split into submodules, public surface intact, file-size clean"
