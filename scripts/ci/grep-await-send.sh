#!/usr/bin/env bash
# Lint: fail if the AgentEvent transport is constructed UNBOUNDED.
#
# History. This script used to ban `.send(...).await` on AgentEvent producers,
# on the premise that "the AgentEvent channel is unbounded, so send() is
# synchronous". That premise was retired by TASK-AGS-102 (d5e8ec1a2,
# 2026-07-26, "restore bounded lossless event delivery"), which deliberately
# converted the transport to a bounded `mpsc::Sender<TimestampedEvent>` so a
# full channel applies backpressure instead of dropping events.
#
# Awaiting a bounded sender is now the REQUIRED shape, asserted in three
# places:
#   * crates/archon-core/src/agent/events.rs  (the call itself)
#   * tests/task_ags_102.rs                   (agent_event_send_awaits_capacity)
#   * tests/tc_arch_05_grep_agent_event_send.rs
# Left unchanged, this lint contradicted all three and could never pass.
#
# The real hazard the old lint pointed at survives the change: a bounded send
# awaited while the agent lock is held deadlocks the TUI. That is a
# lock-ordering property this grep cannot see, and it is covered by
# TC-ARCH-04's backpressure test rather than by pattern matching.
#
# What this lint guards now is the invariant that DID survive: the transport
# must stay bounded. A silent regression to `unbounded_channel` would restore
# unbounded memory growth and make the `.await` sites misleading, so those
# constructions are what fail here.
#
# Escape hatch (#230): add a marker comment on the line IMMEDIATELY PRECEDING
# the offending line to suppress a single site:
#     // agent-event-tx-lint: ignore — bespoke non-AgentEvent transport
#     let (tx, rx) = mpsc::unbounded_channel::<TimestampedEvent>();
# Use sparingly. The marker is matched as a case-sensitive substring and MUST
# sit on exactly the line directly above; a two-line gap is not detected.
set -euo pipefail

ROOT="${TUI_GREP_ROOT:-crates/ src/}"

if ! command -v rg >/dev/null 2>&1; then
    echo "ERROR: ripgrep (rg) not found on PATH" >&2
    exit 2
fi

# Build ROOT arg as array so paths with spaces survive and directory-missing
# errors surface instead of being masked by `|| true`.
read -r -a ROOT_ARR <<<"$ROOT"
for r in "${ROOT_ARR[@]}"; do
    if [[ ! -e "$r" ]]; then
        echo "ERROR: grep-await-send ROOT '$r' does not exist" >&2
        exit 2
    fi
done

# Unbounded constructions of the Agent event transport. Both the channel
# factory and the sender type are matched, so neither half of a regression
# slips through on its own.
UNBOUNDED_PATTERN='(unbounded_channel\s*::\s*<\s*(TimestampedEvent|AgentEvent)\s*>|UnboundedSender\s*<\s*(TimestampedEvent|AgentEvent)\s*>|UnboundedReceiver\s*<\s*(TimestampedEvent|AgentEvent)\s*>)'

# Production sources only. Unlike the `.send().await` ban this replaced, an
# unbounded channel inside a test is not a latent production bug: the TUI load
# and channel-memory harnesses build their own unbounded channels on purpose to
# measure growth, and `preserve_no_await_on_send_gate.rs` asserts on the
# forbidden type as a string. Those are fixtures, not the shipped transport.
HITS=$(rg -n --no-heading --with-filename --type rust \
    --glob '!**/tests/**' --glob '!**/*_tests.rs' --glob '!**/*_test.rs' \
    "$UNBOUNDED_PATTERN" "${ROOT_ARR[@]}" 2>&1) || {
    rc=$?
    if [[ $rc -eq 1 ]]; then
        HITS=""
    else
        echo "ERROR: rg scan failed (rc=$rc):" >&2
        echo "$HITS" >&2
        exit 2
    fi
}

# Filter out hits whose immediately-preceding line carries the ignore marker.
FILTERED_HITS=""
if [[ -n "$HITS" ]]; then
    while IFS= read -r line; do
        [[ -z "$line" ]] && continue
        # rg -n format: <path>:<lineno>:<text>
        path="${line%%:*}"
        rest="${line#*:}"
        lineno="${rest%%:*}"
        # Pass through any line that does not match the path:lineno:text shape.
        if [[ ! "$lineno" =~ ^[0-9]+$ ]]; then
            FILTERED_HITS+="$line"$'\n'
            continue
        fi
        prev_line=""
        if [[ "$lineno" -gt 1 && -f "$path" ]]; then
            prev_line=$(sed -n "$((lineno - 1))p" "$path" 2>/dev/null || echo "")
        fi
        if [[ "$prev_line" == *"agent-event-tx-lint: ignore"* ]]; then
            continue
        fi
        FILTERED_HITS+="$line"$'\n'
    done <<<"$HITS"
    FILTERED_HITS="${FILTERED_HITS%$'\n'}"
fi

if [[ -n "$FILTERED_HITS" ]]; then
    echo "FAIL: unbounded AgentEvent transport detected — TASK-AGS-102 requires a bounded sender"
    echo "$FILTERED_HITS"
    exit 1
fi

echo "OK: AgentEvent transport is bounded"
exit 0
