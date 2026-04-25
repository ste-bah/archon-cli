#!/usr/bin/env bash
# check-tui-file-sizes.sh — archon-tui-scoped file-size ratchet gate.
#
# Walks crates/archon-tui/src/**/*.rs and fails if any file exceeds
# TUI_FILE_SIZE_LIMIT (default 500), except paths listed in
# scripts/check-tui-file-sizes.allowlist.
#
# Enforces NFR-TUI-QUAL-001, NFR-TUI-MOD-001, EC-TUI-018, ERR-TUI-004.
#
# Usage:   bash scripts/check-tui-file-sizes.sh
# Env:     TUI_FILE_SIZE_LIMIT (int, default 500)
# Exit:    0 if every non-allowlisted file is <= limit, 1 otherwise.

set -euo pipefail

THRESHOLD="${TUI_FILE_SIZE_LIMIT:-500}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

ALLOWLIST_FILE="scripts/check-tui-file-sizes.allowlist"

declare -A ALLOW
if [ -f "$ALLOWLIST_FILE" ]; then
  while IFS= read -r line || [ -n "$line" ]; do
    entry="${line%%#*}"
    entry="$(echo "$entry" | awk '{$1=$1;print}')"
    [ -z "$entry" ] && continue
    # Use intermediate variable to avoid set -u triggering on
    # path components inside the subscript expression.
    key="$entry"
    ALLOW["$key"]=1
  done < "$ALLOWLIST_FILE"
fi

TOTAL=0
OVER=0
ALLOWED=0
OFFENDERS_OUT=""
ALLOWED_OUT=""

while IFS= read -r f; do
  rel="${f#./}"
  TOTAL=$((TOTAL + 1))
  lines=$(wc -l < "$f" | awk '{print $1}')
  if [ "$lines" -gt "$THRESHOLD" ]; then
    if [ -n "${ALLOW[$rel]+set}" ]; then
      ALLOWED=$((ALLOWED + 1))
      ALLOWED_OUT+=$(printf '  %6d  %s (allowlisted)\n' "$lines" "$rel")$'\n'
    else
      OVER=$((OVER + 1))
      # GHA annotation for inline PR diagnostics.
      printf '::error file=%s::%s is %d lines (>=%d)\n' "$rel" "$rel" "$lines" "$THRESHOLD"
      OFFENDERS_OUT+=$(printf '  %6d  %s\n' "$lines" "$rel")$'\n'
    fi
  fi
done < <(find crates/archon-tui/src -type f -name '*.rs' | LC_ALL=C sort)

if [ -n "$OFFENDERS_OUT" ]; then
  printf 'TuiFileSizeGuard: offenders (> %d lines):\n' "$THRESHOLD"
  printf '%s' "$OFFENDERS_OUT"
fi
if [ -n "$ALLOWED_OUT" ]; then
  printf 'TuiFileSizeGuard: allowlisted (> %d lines, phase-3 ratchet):\n' "$THRESHOLD"
  printf '%s' "$ALLOWED_OUT"
fi

printf 'TuiFileSizeGuard: %d files checked, %d over %d, %d allowlisted\n' \
  "$TOTAL" "$OVER" "$THRESHOLD" "$ALLOWED"

if [ "$OVER" -gt 0 ]; then
  exit 1
fi
exit 0