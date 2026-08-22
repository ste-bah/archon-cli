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
JSCPD_PACKAGE="${JSCPD_PACKAGE:-jscpd@5.0.4}"
npx --yes --package "$JSCPD_PACKAGE" jscpd \
  --format rust \
  --min-lines "$MIN_LINES" \
  --threshold "$THRESHOLD" \
  --reporters json \
  --output "$REPORT_DIR" \
  "$TUI_SRC"

# jscpd does NOT exit non-zero when the threshold is exceeded — parse the report.
#
# Everything below refuses to report PASS unless it can show what it measured.
# This block used to end with an unconditional `printf PASS; exit 0`, so a run
# that produced no report printed "no report file generated" and then passed, and
# a run without python3 printed the word "unknown" and then passed. A gate that
# measured nothing is broken, not clean.
# See docs/postmortem/0004-a-swallowed-failure-reported-an-absence-of-problems.md.

if [ ! -f "$REPORT_FILE" ]; then
  printf 'TuiDuplicationGuard: FAIL - jscpd wrote no report at %s\n' "$REPORT_FILE" >&2
  printf '  jscpd exited 0 but produced nothing to measure, so duplication is\n' >&2
  printf '  UNKNOWN, not zero. Check the jscpd invocation above.\n' >&2
  exit 1
fi

# One parse, and it must succeed. A missing python3, a truncated report, or a
# schema change now aborts under `set -e` instead of degrading to a printed word.
SUMMARY=$(python3 -c "
import json
with open('${REPORT_FILE}') as f:
    data = json.load(f)
total = data['statistics']['total']
print(f\"{float(total['percentage']):.2f} {int(total['sources'])} {int(total['lines'])}\")
")
DUP_PCT=$(printf '%s' "$SUMMARY" | cut -d' ' -f1)
SOURCES=$(printf '%s' "$SUMMARY" | cut -d' ' -f2)
LINES=$(printf '%s' "$SUMMARY" | cut -d' ' -f3)

# The denominator. 0% duplication measured over zero files is not a pass — it is
# the same empty scan in a different disguise.
if [ "$SOURCES" -eq 0 ] || [ "$LINES" -eq 0 ]; then
  printf 'TuiDuplicationGuard: FAIL - report covers %s files / %s lines\n' "$SOURCES" "$LINES" >&2
  printf '  The scan target (%s) is empty or unreadable.\n' "$TUI_SRC" >&2
  exit 1
fi

printf 'TuiDuplicationGuard: duplication = %s%% over %s files / %s lines (threshold = %d%%)\n' \
  "$DUP_PCT" "$SOURCES" "$LINES" "$THRESHOLD"

if python3 -c "import sys; sys.exit(0 if ${DUP_PCT} > ${THRESHOLD} else 1)"; then
  printf '::error::archon-tui code duplication exceeds %d%% threshold\n' "$THRESHOLD"
  printf 'TuiDuplicationGuard: FAIL\n' >&2
  exit 1
fi

printf 'TuiDuplicationGuard: PASS\n'
exit 0
