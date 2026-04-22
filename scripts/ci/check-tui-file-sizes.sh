#!/usr/bin/env bash
# Lint: fail CI when any file under $TUI_SRC_ROOT exceeds $MAX_LINES.
# Spec: 02-technical-spec.md line 1252 (lint_script: check-tui-file-sizes.sh)
# Implements: AC-OBSERVABILITY-01, EC-TUI-018, TC-TUI-OBSERVABILITY-01
#
# Line counting uses awk's NR rather than `wc -l` because POSIX `wc -l`
# counts newline terminators, not lines — a 501-line file that lacks a
# trailing newline gets reported as 500 and slips under a 500-ceiling gate.
# awk's END{print NR} counts records (a record is any string ending in RS
# OR trailing at EOF), which matches the way humans count lines.
set -euo pipefail

MAX="${MAX_LINES:-500}"
ROOT="${TUI_SRC_ROOT:-crates/archon-tui/src}"
fail=0

if [[ ! -d "$ROOT" ]]; then
    echo "ERROR: TUI_SRC_ROOT '$ROOT' does not exist" >&2
    exit 2
fi

while IFS= read -r f; do
    # awk 'END{print NR}' counts logical records; handles files with or
    # without trailing newline identically.
    lines=$(awk 'END{print NR}' "$f")
    if [[ "$lines" -gt "$MAX" ]]; then
        echo "FAIL $f: $lines > $MAX"
        fail=1
    fi
done < <(find "$ROOT" -name '*.rs' -type f)

if [[ $fail -eq 0 ]]; then
    echo "OK: all files <= $MAX lines"
fi
exit $fail
