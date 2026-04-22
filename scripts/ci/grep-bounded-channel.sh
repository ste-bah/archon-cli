#!/usr/bin/env bash
# Lint: fail if any source code declares a BOUNDED mpsc channel for AgentEvent.
# Spec: 02-technical-spec.md line 1131 — "grep '\"mpsc::channel::<AgentEvent>\"' -> 0 hits"
# Use unbounded_channel instead (NFR-TUI-MOD-002 / SPEC-TUI-EVENTCHANNEL).
set -euo pipefail

ROOT="${TUI_GREP_ROOT:-crates/ src/}"
PATTERN='mpsc::channel::<AgentEvent>'

if ! command -v rg >/dev/null 2>&1; then
    echo "ERROR: ripgrep (rg) not found on PATH" >&2
    exit 2
fi

# Split ROOT on whitespace so multi-dir defaults work.
# shellcheck disable=SC2086
HITS=$(rg -n --no-heading "$PATTERN" $ROOT 2>/dev/null || true)

if [[ -n "$HITS" ]]; then
    echo "FAIL: bounded AgentEvent channels detected — use mpsc::unbounded_channel instead"
    echo "$HITS"
    exit 1
fi

echo "OK: no bounded AgentEvent channels"
exit 0
