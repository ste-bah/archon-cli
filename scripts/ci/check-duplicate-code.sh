#!/usr/bin/env bash
# Duplicate code gate - fail CI if duplication % >= threshold (default 5).
# Spec: 02-technical-spec.md line 1130, NFR-TUI-MOD-003
# Implements: AC-OBSERVABILITY-03, TC-TUI-OBSERVABILITY-04
set -euo pipefail

TARGET="${JSCPD_TARGET_DIR:-crates/archon-tui/src}"
THRESHOLD="${JSCPD_THRESHOLD:-5}"
REPORT_DIR="${JSCPD_REPORT_DIR:-/tmp/jscpd-report}"

if [[ ! -d "$TARGET" ]]; then
    echo "ERROR: target dir '$TARGET' does not exist" >&2
    exit 2
fi

if ! command -v npx >/dev/null 2>&1; then
    echo "ERROR: npx not found on PATH (install Node.js)" >&2
    exit 2
fi

if ! command -v python3 >/dev/null 2>&1; then
    echo "ERROR: python3 not found on PATH" >&2
    exit 2
fi

rm -rf "$REPORT_DIR"
mkdir -p "$REPORT_DIR"

echo "Running jscpd on $TARGET (threshold: ${THRESHOLD}%)..."
# Run jscpd. We parse the JSON report to enforce threshold; jscpd's own exit
# code is informational only.
set +e
npx --yes jscpd@4 \
    --threshold "$THRESHOLD" \
    --reporters json \
    --output "$REPORT_DIR" \
    --format rust \
    --silent \
    "$TARGET" >/tmp/jscpd-stdout.log 2>&1
JSCPD_RC=$?
set -e

REPORT_JSON="$REPORT_DIR/jscpd-report.json"
if [[ ! -f "$REPORT_JSON" ]]; then
    echo "ERROR: jscpd report missing at $REPORT_JSON" >&2
    echo "jscpd exit $JSCPD_RC; stdout:" >&2
    tail -20 /tmp/jscpd-stdout.log >&2 || true
    exit 2
fi

PERCENT=$(python3 -c "
import json, sys
try:
    with open('$REPORT_JSON') as f:
        d = json.load(f)
    pct = d.get('statistics', {}).get('total', {}).get('percentage', 0.0)
    print(pct)
except Exception as e:
    sys.stderr.write(f'ERROR: failed to parse jscpd report: {e}\n')
    sys.exit(2)
")

printf 'jscpd: %s%% duplication (threshold %s%%)\n' "$PERCENT" "$THRESHOLD"

# Compare using python for float correctness.
EXCEEDS=$(python3 -c "
import sys
try:
    print(1 if float('$PERCENT') >= float('$THRESHOLD') else 0)
except Exception as e:
    sys.stderr.write(f'ERROR: failed to compare threshold: {e}\n')
    sys.exit(2)
")

if [[ "$EXCEEDS" == "1" ]]; then
    echo "FAIL: duplication exceeds threshold"
    exit 1
fi

echo "OK: duplication below threshold"
exit 0
