#!/usr/bin/env bash
set -uo pipefail

hits=$(
  rg "200_000|200000" crates src -g '*.rs' --glob '!**/tests/**' 2>/dev/null \
    | grep -v "#\\[cfg(test\\|#\\[tokio::test\\|audit-allow" \
    | grep "context_window\\|context_limit\\|window.*200" || true
)

if [ -n "$hits" ]; then
  printf '%s\n' "$hits"
  echo "check-context-window-literals: hardcoded context-window literal found" >&2
  exit 1
fi

echo "check-context-window-literals: clean"
