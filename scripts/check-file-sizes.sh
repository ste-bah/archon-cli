#!/usr/bin/env bash
# check-file-sizes.sh — FileSizeGuard for NFR-FOR-D4-MAINTAINABILITY.
#
# Walks the repo for Rust and web source files (excluding generated/build
# directories)
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

# Normalize allowlist entries into a temp file (paths are repo-relative).
# Keep this Bash 3.2-compatible for macOS runners and developer machines.
ALLOWLIST_NORMALIZED="$(mktemp)"
trap 'rm -f "$ALLOWLIST_NORMALIZED"' EXIT
if [ -f "$ALLOWLIST_FILE" ]; then
  sed 's/#.*//' "$ALLOWLIST_FILE" | awk '{$1=$1;print}' | sed '/^$/d' > "$ALLOWLIST_NORMALIZED"
else
  : > "$ALLOWLIST_NORMALIZED"
fi

TOTAL=0
OVER=0
ALLOWED=0
OFFENDERS_OUT=""
ALLOWED_OUT=""

# Walk every checked source file in the repo, skipping build + fixture dirs.
while IFS= read -r f; do
  # Strip leading ./
  rel="${f#./}"
  TOTAL=$((TOTAL + 1))
  lines=$(wc -l < "$f" | awk '{print $1}')
  if [ "$lines" -gt "$THRESHOLD" ]; then
    if grep -Fxq -- "$rel" "$ALLOWLIST_NORMALIZED"; then
      ALLOWED=$((ALLOWED + 1))
      ALLOWED_OUT+=$(printf '  %6d  %s (allowlisted)\n' "$lines" "$rel")$'\n'
    else
      OVER=$((OVER + 1))
      OFFENDERS_OUT+=$(printf '  %6d  %s\n' "$lines" "$rel")$'\n'
    fi
  fi
done < <(find . -type f \( \
    -name '*.rs' \
    -o -name '*.ts' \
    -o -name '*.tsx' \
    -o -name '*.css' \
  \) \
  -not -path '*/target/*' \
  -not -path '*/.cargo/*' \
  -not -path '*/node_modules/*' \
  -not -path '*/web/dist/*' \
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
