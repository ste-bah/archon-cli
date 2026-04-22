#!/usr/bin/env bash
# Lint: fail if any code awaits on agent_event_tx.send(...) — unbounded channels are sync.
# Spec: 02-technical-spec.md line 1132 — "grep 'agent_event_tx\.send\(.*\)\.await' -> 0 hits"
set -euo pipefail

ROOT="${TUI_GREP_ROOT:-crates/ src/}"
PATTERN='agent_event_tx\.send\([^)]*\)\.await'

if ! command -v rg >/dev/null 2>&1; then
    echo "ERROR: ripgrep (rg) not found on PATH" >&2
    exit 2
fi

# shellcheck disable=SC2086
HITS=$(rg -n --no-heading "$PATTERN" $ROOT 2>/dev/null || true)

if [[ -n "$HITS" ]]; then
    echo "FAIL: .send(...).await detected on agent_event_tx — unbounded channels are sync"
    echo "$HITS"
    exit 1
fi

echo "OK: no .send(...).await on agent_event_tx"
exit 0
