#!/usr/bin/env bash
# Gate-1 structural check for TASK-P0-B.6b (#185) PushNotification tool.
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

if [[ ! -f "$REPO_ROOT/crates/archon-tools/src/push_notification.rs" ]]; then
    echo "RED: crates/archon-tools/src/push_notification.rs does not exist"
    FAIL=1
else
    echo "OK: push_notification.rs present"
fi

chk "PushNotificationTool struct" \
    "crates/archon-tools/src/push_notification.rs" \
    "pub struct PushNotificationTool"

chk "impl Tool for PushNotificationTool" \
    "crates/archon-tools/src/push_notification.rs" \
    "impl Tool for PushNotificationTool"

chk "archon-tools lib.rs declares pub mod push_notification" \
    "crates/archon-tools/src/lib.rs" \
    "pub mod push_notification"

chk "dispatch.rs registers PushNotificationTool" \
    "crates/archon-core/src/dispatch.rs" \
    "push_notification::PushNotificationTool"

chk "input_schema declares title as required" \
    "crates/archon-tools/src/push_notification.rs" \
    "\"title\""

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: P0-B.6b PushNotification tool surfaces present"
