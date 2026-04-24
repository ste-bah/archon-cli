#!/usr/bin/env bash
# Gate-1 structural check for TASK-P0-B.6a (#184) Monitor tool.
# Verifies:
#   1. crates/archon-tools/src/monitor.rs exists
#   2. MonitorTool struct + impl Tool
#   3. archon-tools lib.rs declares pub mod monitor
#   4. dispatch.rs registers MonitorTool
#   5. input_schema declares required command
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

if [[ ! -f "$REPO_ROOT/crates/archon-tools/src/monitor.rs" ]]; then
    echo "RED: crates/archon-tools/src/monitor.rs does not exist"
    FAIL=1
else
    echo "OK: monitor.rs present"
fi

chk "MonitorTool struct" \
    "crates/archon-tools/src/monitor.rs" \
    "pub struct MonitorTool"

chk "impl Tool for MonitorTool" \
    "crates/archon-tools/src/monitor.rs" \
    "impl Tool for MonitorTool"

chk "archon-tools lib.rs declares pub mod monitor" \
    "crates/archon-tools/src/lib.rs" \
    "pub mod monitor"

chk "dispatch.rs registers MonitorTool" \
    "crates/archon-core/src/dispatch.rs" \
    "monitor::MonitorTool"

chk "input_schema declares command as required" \
    "crates/archon-tools/src/monitor.rs" \
    "\"command\""

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: P0-B.6a Monitor tool surfaces present"
