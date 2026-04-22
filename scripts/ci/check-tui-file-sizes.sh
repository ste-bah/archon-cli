#!/usr/bin/env bash
# Lint: fail CI when any file under $TUI_SRC_ROOT exceeds $MAX_LINES.
# Spec: 02-technical-spec.md line 1252 (lint_script: check-tui-file-sizes.sh)
# Implements: AC-OBSERVABILITY-01, EC-TUI-018, TC-TUI-OBSERVABILITY-01
set -euo pipefail

MAX="${MAX_LINES:-500}"
ROOT="${TUI_SRC_ROOT:-crates/archon-tui/src}"
fail=0

if [[ ! -d "$ROOT" ]]; then
    echo "ERROR: TUI_SRC_ROOT '$ROOT' does not exist" >&2
    exit 2
fi

while IFS= read -r f; do
    lines=$(wc -l <"$f")
    if [ "$lines" -gt "$MAX" ]; then
        echo "FAIL $f: $lines > $MAX"
        fail=1
    fi
done < <(find "$ROOT" -name '*.rs')

if [ "$fail" -eq 0 ]; then
    echo "OK: all files <= $MAX lines"
fi
exit $fail
