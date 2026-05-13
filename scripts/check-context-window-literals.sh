#!/usr/bin/env bash
set -uo pipefail

literal_hits=$(
  rg "200_000|200000" crates src -g '*.rs' --glob '!**/tests/**' 2>/dev/null \
    | grep -v "#\\[cfg(test\\|#\\[tokio::test\\|audit-allow" \
    | grep "context_window\\|context_limit\\|window.*200" || true
)
budget_hits=$(
  rg "model_context_window\\s*\\*\\s*4\\s*/\\s*5|context_window\\s*\\*\\s*4\\s*/\\s*5|4\\.0\\s*/\\s*5\\.0" \
    crates src -g '*.rs' --glob '!**/tests/**' 2>/dev/null \
    | grep -v "#\\[cfg(test\\|#\\[tokio::test\\|audit-allow" || true
)

hits=$(printf '%s\n%s\n' "$literal_hits" "$budget_hits" | sed '/^$/d')

if [ -n "$hits" ]; then
  printf '%s\n' "$hits"
  echo "check-context-window-literals: hardcoded context-window literal found" >&2
  exit 1
fi

echo "check-context-window-literals: clean"
