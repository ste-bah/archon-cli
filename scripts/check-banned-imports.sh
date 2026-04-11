#!/usr/bin/env bash
# check-banned-imports.sh — Banned-Imports Guard (REQ-FOR-PRESERVE-D8).
#
# Loads scripts/check-banned-imports.patterns (ERE regex per non-comment
# line) and greps crates/, src/, tests/ for each. Reports every hit.
# Allowlist: scripts/check-banned-imports.allowlist holds `pattern: path`
# tuples; a hit whose pattern AND path match is skipped.
#
# Usage: bash scripts/check-banned-imports.sh
# Exit:  0 on clean HEAD, 1 on any hit.

set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

PATTERN_FILE="scripts/check-banned-imports.patterns"
ALLOWLIST_FILE="scripts/check-banned-imports.allowlist"
[ -f "$PATTERN_FILE" ] || { echo "missing $PATTERN_FILE" >&2; exit 2; }

# Strip comments + trim an allowlist/patterns line.
trim() { echo "${1%%#*}" | awk '{$1=$1;print}'; }

declare -A ALLOW
if [ -f "$ALLOWLIST_FILE" ]; then
  while IFS= read -r raw; do
    line=$(trim "$raw"); [ -z "$line" ] && continue
    pat="${line%%: *}"; pth="${line#*: }"
    [ -z "$pat" ] || [ -z "$pth" ] || [ "$pat" = "$line" ] && continue
    ALLOW["${pat}"$'\t'"${pth}"]=1
  done < "$ALLOWLIST_FILE"
fi

SCAN_ROOTS=()
for d in crates src tests; do [ -d "$d" ] && SCAN_ROOTS+=("$d"); done
[ "${#SCAN_ROOTS[@]}" -eq 0 ] && { echo "no scan roots" >&2; exit 0; }
EXCLUDES=(--exclude-dir=target --exclude-dir=baseline)

HITS=0; ALLOWED=0; HIT_LINES=""
while IFS= read -r raw; do
  pat=$(trim "$raw"); [ -z "$pat" ] && continue
  out=$(LC_ALL=C grep -rnE "${EXCLUDES[@]}" -- "$pat" "${SCAN_ROOTS[@]}" 2>/dev/null | LC_ALL=C sort)
  [ -z "$out" ] && continue
  while IFS= read -r line; do
    file="${line%%:*}"
    if [ -n "${ALLOW["${pat}"$'\t'"${file}"]+set}" ]; then
      ALLOWED=$((ALLOWED + 1)); continue
    fi
    HIT_LINES+="BANNED: ${pat}  found at ${line}"$'\n'
    HITS=$((HITS + 1))
  done <<< "$out"
done < "$PATTERN_FILE"

NP=$(grep -cE '^[[:space:]]*[^#[:space:]]' "$PATTERN_FILE")
if [ "$HITS" -gt 0 ]; then
  printf '%s' "$HIT_LINES"
  printf 'check-banned-imports: %d hit(s), %d allowlisted, %d patterns, %d scan roots\n' \
    "$HITS" "$ALLOWED" "$NP" "${#SCAN_ROOTS[@]}" >&2
  exit 1
fi
printf 'check-banned-imports: clean (%d patterns, %d allowlisted, %d scan roots)\n' \
  "$NP" "$ALLOWED" "${#SCAN_ROOTS[@]}" >&2
exit 0
