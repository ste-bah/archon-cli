#!/usr/bin/env bash
# Self-test for grep-await-send.sh.
#
# Formerly also covered grep-bounded-channel.sh, which required the AgentEvent
# transport to be UNBOUNDED. TASK-AGS-102 (d5e8ec1a2) reversed that: the
# transport is bounded on purpose so a full channel backpressures instead of
# dropping events. That script and its cases were retired rather than left
# mandating the opposite of the shipped design.
#
# 2 cases:
#   A. unbounded hit   — tmp with mpsc::unbounded_channel::<AgentEvent>() -> exit 1
#   B. bounded clean   — tmp with mpsc::channel::<AgentEvent>(256)        -> exit 0
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AWAITED="${SCRIPT_DIR}/../grep-await-send.sh"

if [[ ! -x "$AWAITED" ]]; then
    echo "FAIL: grep-await-send.sh not executable at $AWAITED"
    exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

run_case() {
    local label="$1" script="$2" content="$3" expect_rc="$4" expect_stdout_contains="$5"
    local case_dir="$TMP/${label}"
    mkdir -p "$case_dir"
    printf '%s\n' "$content" > "$case_dir/file.rs"

    set +e
    OUT=$(TUI_GREP_ROOT="$case_dir" bash "$script" 2>&1)
    RC=$?
    set -e

    if [ "$RC" -ne "$expect_rc" ]; then
        echo "CASE $label FAIL: expected rc=$expect_rc, got rc=$RC"
        echo "Output: $OUT"
        exit 1
    fi

    if [ -n "$expect_stdout_contains" ] && ! echo "$OUT" | grep -qF "$expect_stdout_contains"; then
        echo "CASE $label FAIL: output missing expected substring '$expect_stdout_contains'"
        echo "Output: $OUT"
        exit 1
    fi

    echo "PASS $label (rc=$RC)"
}

# Case A: unbounded AgentEvent transport is the banned construction.
run_case "A-unbounded-hit" "$AWAITED" 'let (tx, rx) = mpsc::unbounded_channel::<AgentEvent>();' 1 "unbounded_channel"

# Case B: a bounded channel — and an awaited send on it — is the required shape.
run_case "B-bounded-ok" "$AWAITED" 'let (tx, rx) = mpsc::channel::<AgentEvent>(256);
self.event_tx.send(timestamped).await.ok();' 0 "OK"

echo "ALL TESTS PASSED"
