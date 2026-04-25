#!/usr/bin/env bash
# Regression test for #230 — grep-await-send.sh false-positive escape hatch.
#
# The lint must:
#   - SKIP a `.send(...).await` whose preceding line contains the marker
#       // agent-event-tx-lint: ignore
#   - STILL FLAG a `.send(...).await` without the marker (true positives
#     remain caught — the marker is opt-in, not a global disable).
set -euo pipefail

WORKTREE="${WORKTREE:-$(git rev-parse --show-toplevel 2>/dev/null)}"
if [[ -z "$WORKTREE" || ! -f "$WORKTREE/scripts/ci/grep-await-send.sh" ]]; then
    echo "ERROR: WORKTREE not set or missing grep-await-send.sh"
    exit 2
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# Fixture 1: marker present — must NOT trigger lint
cat > "$TMP/annotated.rs" <<'EOF'
struct AgentEvent;
async fn make_call() {
    // agent-event-tx-lint: ignore — channel holds OrchestratorEvent
    let _ = event_tx.send(SomeOtherEvent::X).await;
}
EOF

# Fixture 2: no marker — MUST trigger lint
cat > "$TMP/unannotated.rs" <<'EOF'
struct AgentEvent;
async fn make_call() {
    let _ = event_tx.send(BadPayload::Y).await;
}
EOF

cd "$TMP"
LINT_LOG=$(mktemp)
EXIT_CODE=0
TUI_GREP_ROOT="." bash "$WORKTREE/scripts/ci/grep-await-send.sh" > "$LINT_LOG" 2>&1 || EXIT_CODE=$?

# Unannotated fixture present → lint MUST exit 1
if [[ $EXIT_CODE -ne 1 ]]; then
    echo "FAIL: expected exit 1 (unannotated true positive), got $EXIT_CODE"
    cat "$LINT_LOG"
    exit 1
fi

# True-positive guard: MUST mention unannotated.rs
if ! grep -q "unannotated.rs" "$LINT_LOG"; then
    echo "FAIL: missed unannotated true positive"
    cat "$LINT_LOG"
    exit 1
fi

# False-positive escape hatch guard: MUST NOT mention annotated.rs
if grep -q "annotated.rs:[0-9]" "$LINT_LOG" && ! grep -qE "unannotated\.rs:[0-9]+" <(grep "annotated.rs:[0-9]" "$LINT_LOG"); then
    # The grep above is intentionally strict — "annotated.rs" is a substring of
    # "unannotated.rs", so we must allow unannotated lines but reject lines
    # that are PURELY annotated.rs hits.
    if grep -E '(^| )annotated\.rs:[0-9]+' "$LINT_LOG" >/dev/null; then
        echo "FAIL: marker did not suppress annotated.rs hit (escape hatch broken)"
        cat "$LINT_LOG"
        exit 1
    fi
fi

# Tighter check using path component — annotated.rs as a discrete name
if grep -E '(^|/)annotated\.rs:[0-9]+' "$LINT_LOG" >/dev/null; then
    echo "FAIL: marker did not suppress annotated.rs hit (escape hatch broken)"
    cat "$LINT_LOG"
    exit 1
fi

echo "PASS: marker-based escape hatch works"
exit 0
