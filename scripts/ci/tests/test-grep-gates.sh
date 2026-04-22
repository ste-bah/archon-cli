#!/usr/bin/env bash
# Self-test for grep-bounded-channel.sh and grep-await-send.sh
# 4 cases:
#   A. bounded hit     — tmp with mpsc::channel::<AgentEvent>(100) -> bounded script exit 1
#   B. bounded clean   — tmp with mpsc::unbounded_channel::<AgentEvent>() -> bounded script exit 0
#   C. await hit       — tmp with agent_event_tx.send(e).await -> await script exit 1
#   D. await clean     — tmp without that pattern -> await script exit 0
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BOUNDED="${SCRIPT_DIR}/../grep-bounded-channel.sh"
AWAITED="${SCRIPT_DIR}/../grep-await-send.sh"

if [[ ! -x "$BOUNDED" ]]; then
    echo "FAIL: grep-bounded-channel.sh not executable at $BOUNDED"
    exit 1
fi
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

# Case A: bounded channel hit
run_case "A-bounded-hit" "$BOUNDED" 'let (tx, rx) = mpsc::channel::<AgentEvent>(100);' 1 "mpsc::channel::<AgentEvent>"

# Case B: unbounded clean
run_case "B-bounded-ok" "$BOUNDED" 'let (tx, rx) = mpsc::unbounded_channel::<AgentEvent>();' 0 "OK"

# Case C: await on send hit
run_case "C-await-hit" "$AWAITED" 'agent_event_tx.send(event).await.unwrap();' 1 "agent_event_tx"

# Case D: await clean
run_case "D-await-ok" "$AWAITED" 'agent_event_tx.send(event);' 0 "OK"

echo "ALL TESTS PASSED"
