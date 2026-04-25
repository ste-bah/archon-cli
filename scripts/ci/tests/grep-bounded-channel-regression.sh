#!/usr/bin/env bash
# Regression test for #229 — grep-bounded-channel.sh must:
#   - NOT match `unbounded_channel::<AgentEvent>` (correct, unbounded form)
#   - MUST match `mpsc::channel::<AgentEvent>` (bounded form, the violation)
#
# Failure mode pre-fix: P1 lacked `\b` before `channel`, so `unbounded_channel`
# matched as a substring (false positive blocking the tui-observability gate).
set -euo pipefail

WORKTREE="${WORKTREE:-$(git rev-parse --show-toplevel 2>/dev/null)}"
if [[ -z "$WORKTREE" || ! -f "$WORKTREE/scripts/ci/grep-bounded-channel.sh" ]]; then
    echo "ERROR: WORKTREE not set or missing grep-bounded-channel.sh"
    exit 2
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

cat > "$TMP/unbounded.rs" <<'EOF'
use tokio::sync::mpsc;
fn make_unbounded() {
    let (_tx, _rx) = mpsc::unbounded_channel::<AgentEvent>();
}
EOF

cat > "$TMP/bounded.rs" <<'EOF'
use tokio::sync::mpsc;
fn make_bounded() {
    let (_tx, _rx) = mpsc::channel::<AgentEvent>(256);
}
EOF

cd "$TMP"
LINT_LOG=$(mktemp)
EXIT_CODE=0
TUI_GREP_ROOT="." bash "$WORKTREE/scripts/ci/grep-bounded-channel.sh" > "$LINT_LOG" 2>&1 || EXIT_CODE=$?

# Bounded fixture present → lint MUST exit 1
if [[ $EXIT_CODE -ne 1 ]]; then
    echo "FAIL: expected exit 1 (bounded match), got $EXIT_CODE"
    cat "$LINT_LOG"
    exit 1
fi

# False-positive guard: must NOT mention unbounded.rs
if grep -q "unbounded.rs" "$LINT_LOG"; then
    echo "FAIL: false positive — matched unbounded_channel::<AgentEvent>"
    cat "$LINT_LOG"
    exit 1
fi

# True-positive guard: MUST mention bounded.rs
if ! grep -q "bounded.rs" "$LINT_LOG"; then
    echo "FAIL: missed true positive — bounded mpsc::channel::<AgentEvent>"
    cat "$LINT_LOG"
    exit 1
fi

echo "PASS: lint distinguishes bounded from unbounded"
exit 0
