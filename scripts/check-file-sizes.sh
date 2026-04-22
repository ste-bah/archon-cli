#!/usr/bin/env bash
# check-file-sizes.sh — FileSizeGuard for NFR-FOR-D4-MAINTAINABILITY.
#
# Walks the repo for *.rs files (excluding target/, .cargo/, tests/fixtures/)
# and fails if any file exceeds 500 lines, except those in
# scripts/check-file-sizes.allowlist.
#
# Usage: bash scripts/check-file-sizes.sh
# Exit:  0 if every non-allowlisted file is <=500 lines, 1 otherwise.

set -uo pipefail

THRESHOLD=500
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

ALLOWLIST_FILE="scripts/check-file-sizes.allowlist"

# Load allowlist into an associative array (paths are repo-relative).
declare -A ALLOW
if [ -f "$ALLOWLIST_FILE" ]; then
  while IFS= read -r line; do
    # Strip comments and whitespace.
    entry="${line%%#*}"
    entry="$(echo "$entry" | awk '{$1=$1;print}')"
    [ -z "$entry" ] && continue
    ALLOW["$entry"]=1
  done < "$ALLOWLIST_FILE"
fi

TOTAL=0
OVER=0
ALLOWED=0
OFFENDERS_OUT=""
ALLOWED_OUT=""

# Walk every *.rs in the repo, skipping build + fixture dirs.
while IFS= read -r f; do
  # Strip leading ./
  rel="${f#./}"
  TOTAL=$((TOTAL + 1))
  lines=$(wc -l < "$f" | awk '{print $1}')
  if [ "$lines" -gt "$THRESHOLD" ]; then
    if [ -n "${ALLOW[$rel]+set}" ]; then
      ALLOWED=$((ALLOWED + 1))
      ALLOWED_OUT+=$(printf '  %6d  %s (allowlisted)\n' "$lines" "$rel")$'\n'
    else
      OVER=$((OVER + 1))
      OFFENDERS_OUT+=$(printf '  %6d  %s\n' "$lines" "$rel")$'\n'
    fi
  fi
done < <(find . -type f -name '*.rs' \
  -not -path '*/target/*' \
  -not -path '*/.cargo/*' \
  -not -path '*/tests/fixtures/*' \
  | LC_ALL=C sort)

if [ -n "$OFFENDERS_OUT" ]; then
  printf 'FileSizeGuard: offenders (> %d lines):\n' "$THRESHOLD"
  printf '%s' "$OFFENDERS_OUT"
fi
if [ -n "$ALLOWED_OUT" ]; then
  printf 'FileSizeGuard: allowlisted (> %d lines, grandfathered):\n' "$THRESHOLD"
  printf '%s' "$ALLOWED_OUT"
fi

printf 'FileSizeGuard: %d files checked, %d over %d, %d allowlisted\n' \
  "$TOTAL" "$OVER" "$THRESHOLD" "$ALLOWED"

if [ "$OVER" -gt 0 ]; then
  exit 1
fi
exit 0
