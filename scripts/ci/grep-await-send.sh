#!/usr/bin/env bash
# Lint: fail if any code awaits on an AgentEvent producer `.send(...).await`.
# The AgentEvent channel is unbounded — send() is synchronous (returns
# Result<(), SendError<_>>), so `.await` on it is a bug either way:
#   * For tokio::sync::mpsc::UnboundedSender::send, `.await` won't compile
#     (send returns a value, not a future) — but refactors that accidentally
#     switch to bounded `mpsc::Sender` DO compile with `.await` and then
#     deadlock the TUI when the agent lock is held across it.
#
# Spec: 02-technical-spec.md line 1132 — "grep 'agent_event_tx\.send\(.*\)\.await' -> 0 hits"
#
# We expand the naive literal-name pattern to catch:
#   - renamed producers (agent_tx, event_tx, tx, producer, self.tx, ...)
#   - multi-line `.send(\n   payload\n).await`
#   - chained `.send(x).await?`, `.send(x).await.ok()`, etc.
#
# Scope is deliberately any send-then-await in source that declares
# `AgentEvent` — `rg --type-add` lets us use `-t rust` to skip generated or
# build artifacts. Test files are included: a `.send(...).await` inside a
# test counts as a latent bug waiting to graduate into prod.
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

# Two-pass narrowing:
#   1) only consider Rust files that mention `AgentEvent` (type-gated scope).
#   2) inside those, flag any `.send(...)` followed by `.await` across lines.
#
# `rg -U` enables multiline matching; `--multiline-dotall` lets `.` cross
# newlines. We grep on the producer-style patterns seen in the codebase
# (tx, producer, sender, self.tx, etc.) rather than any `.send(...).await`
# to avoid false positives on unrelated futures APIs like `request.send().await`
# from reqwest. If a new producer variable name appears it must be added here.
PRODUCER_PATTERN='\b(agent_event_tx|agent_tx|event_tx|events_tx|tx|producer|sender|self\.tx|self\.sender|self\.event_tx|self\.events_tx)\b\s*\.send\s*\([^)]*?(?:\n[^)]*?)*\)\s*\.await'

# Restrict to files in rust type that also import/mention AgentEvent.
AGENT_EVENT_FILES=$(rg -l --type rust 'AgentEvent' "${ROOT_ARR[@]}" 2>&1) || {
    rc=$?
    # rc=1 from rg means "no matches", which is fine (nothing to lint).
    # Any other rc is a real error.
    if [[ $rc -ne 1 ]]; then
        echo "ERROR: rg scanning for AgentEvent failed (rc=$rc):" >&2
        echo "$AGENT_EVENT_FILES" >&2
        exit 2
    fi
    AGENT_EVENT_FILES=""
}

if [[ -z "$AGENT_EVENT_FILES" ]]; then
    echo "OK: no AgentEvent-mentioning files in scope"
    exit 0
fi

# Pipe file list to rg via -F (fixed strings; file names, not a pattern).
# shellcheck disable=SC2086
HITS=$(printf '%s\n' "$AGENT_EVENT_FILES" | xargs -r rg -n --no-heading -U --multiline-dotall "$PRODUCER_PATTERN" 2>&1) || {
    rc=$?
    if [[ $rc -eq 1 ]]; then
        HITS=""
    else
        echo "ERROR: rg multiline scan failed (rc=$rc):" >&2
        echo "$HITS" >&2
        exit 2
    fi
}

if [[ -n "$HITS" ]]; then
    echo "FAIL: producer .send(...).await detected — AgentEvent channel sends are sync"
    echo "$HITS"
    exit 1
fi

echo "OK: no producer .send(...).await in AgentEvent-scope files"
exit 0
