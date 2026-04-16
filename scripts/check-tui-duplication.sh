#!/usr/bin/env bash
# check-tui-duplication.sh — archon-tui code duplication gate.
#
# Runs jscpd (token-based duplication detector) against crates/archon-tui/src.
# Fails if duplication exceeds 5% (NFR-TUI-MOD-003, AC-MOD-04).
#
# Usage:   bash scripts/check-tui-duplication.sh
# Exit:    0 if duplication <= 5%, 1 otherwise.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

TUI_SRC="${TUI_SRC:-crates/archon-tui/src}"
REPORT_DIR="target"
REPORT_FILE="${REPORT_DIR}/jscpd-report.json"
THRESHOLD=5
MIN_LINES=20

mkdir -p "$REPORT_DIR"

# Run jscpd via npx (no global install).
# --reporters json outputs to <output>/jscpd-report.json
# --threshold sets the percentage ceiling
# --min-lines minimum clone length
npx jscpd \
  --pattern "**/*.rs" \
  --min-lines "$MIN_LINES" \
  --threshold "$THRESHOLD" \
  --reporters json \
  --output "$REPORT_DIR" \
  "$TUI_SRC"

# jscpd v4 does NOT exit non-zero when threshold exceeded — parse manually.
if [ -f "$REPORT_FILE" ]; then
  # Extract percentage from the JSON statistics
  DUP_PCT=$(python3 -c "
import json, sys
with open('${REPORT_FILE}') as f:
    data = json.load(f)
stats = data.get('statistics', {})
total = stats.get('total', {})
pct = total.get('percentage', 0)
print(f'{pct:.2f}')
" 2>/dev/null || echo "unknown")
  printf 'TuiDuplicationGuard: duplication = %s%% (threshold = %d%%)\n' "$DUP_PCT" "$THRESHOLD"

  # Manually compare against threshold since jscpd v4 does not exit non-zero on threshold.
  COMPARE_RESULT=$(python3 -c "
import json
with open('${REPORT_FILE}') as f:
    data = json.load(f)
stats = data.get('statistics', {})
total = stats.get('total', {})
pct = total.get('percentage', 0)
if pct > ${THRESHOLD}:
    print('EXCEEDED')
else:
    print('OK')
" 2>/dev/null)

  if [ "$COMPARE_RESULT" = "EXCEEDED" ]; then
    printf '::error::archon-tui code duplication exceeds %d%% threshold\n' "$THRESHOLD"
    exit 1
  fi
else
  printf 'TuiDuplicationGuard: no report file generated\n'
fi

printf 'TuiDuplicationGuard: PASS\n'
exit 0