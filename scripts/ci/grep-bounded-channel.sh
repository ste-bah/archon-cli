#!/usr/bin/env bash
# Lint: fail if any source code declares a BOUNDED mpsc channel for AgentEvent.
# Spec: 02-technical-spec.md line 1131 — "grep '\"mpsc::channel::<AgentEvent>\"' -> 0 hits"
# Use unbounded_channel instead (NFR-TUI-MOD-002 / SPEC-TUI-EVENTCHANNEL).
#
# Pattern expansion vs the literal spec regex:
#   - naked `channel::<AgentEvent>` (when `use tokio::sync::mpsc::channel;` is
#     imported and used bare)
#   - `mpsc::channel::<AgentEvent>` (fully qualified)
#   - whitespace variations: `channel :: < AgentEvent >`
#   - multi-line generics: `channel::<\n    AgentEvent\n>(...)`
#   - type aliases: `type EventTx = mpsc::Sender<AgentEvent>;` combined with
#     `mpsc::channel::<...>(capacity)` inferring AgentEvent — caught via
#     the `Sender<AgentEvent>` paired grep below.
set -euo pipefail

ROOT="${TUI_GREP_ROOT:-crates/ src/}"

if ! command -v rg >/dev/null 2>&1; then
    echo "ERROR: ripgrep (rg) not found on PATH" >&2
    exit 2
fi

# Split ROOT safely and error out if any declared path is missing.
read -r -a ROOT_ARR <<<"$ROOT"
for r in "${ROOT_ARR[@]}"; do
    if [[ ! -e "$r" ]]; then
        echo "ERROR: grep-bounded-channel ROOT '$r' does not exist" >&2
        exit 2
    fi
done

FAIL=0

# Pattern 1: direct bounded-channel invocation for AgentEvent with any
# whitespace / leading module path. Multi-line via -U + --multiline-dotall.
#
# Breakdown:
#   (?:mpsc::)?         -> optional `mpsc::` module prefix
#   channel\s*::\s*<    -> `channel::<` with optional whitespace
#   [\s\S]*?            -> any chars (including newline) lazy
#   \bAgentEvent\b      -> the type we guard
#   [\s\S]*?>           -> close the generic (may be nested)
P1='(?:mpsc::)?channel\s*::\s*<[\s\S]*?\bAgentEvent\b[\s\S]*?>'

# Pattern 2: bounded Sender type mention. `mpsc::Sender<AgentEvent>` is the
# bounded-sender type; the unbounded equivalent is `UnboundedSender<_>`. If
# the code declares `Sender<AgentEvent>` anywhere outside a test double, it
# means somewhere a bounded channel was (or will be) constructed.
P2='\bmpsc::Sender\s*<\s*AgentEvent\b'

run_pattern() {
    local tag="$1" pat="$2"
    local out rc
    out=$(rg -n --no-heading -U --multiline-dotall --type rust "$pat" "${ROOT_ARR[@]}" 2>&1) || rc=$?
    rc=${rc:-0}
    if [[ $rc -eq 1 ]]; then
        return 0  # no matches
    fi
    if [[ $rc -ne 0 ]]; then
        echo "ERROR: rg ($tag) failed (rc=$rc):" >&2
        echo "$out" >&2
        return 2
    fi
    echo "FAIL ($tag):"
    echo "$out"
    return 1
}

if ! run_pattern "channel::<AgentEvent>" "$P1"; then
    FAIL=1
fi
if ! run_pattern "mpsc::Sender<AgentEvent>" "$P2"; then
    FAIL=1
fi

if [[ $FAIL -eq 1 ]]; then
    echo ""
    echo "FAIL: bounded AgentEvent channel or Sender<AgentEvent> detected —"
    echo "      use tokio::sync::mpsc::unbounded_channel + UnboundedSender"
    exit 1
fi

echo "OK: no bounded AgentEvent channels detected"
exit 0
