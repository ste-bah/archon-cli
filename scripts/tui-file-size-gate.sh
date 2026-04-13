#!/usr/bin/env bash
# tui-file-size-gate.sh
# Ratchet-style file-size CI gate for Rust sources.
#
# Reads a JSON allow-list describing grandfathered over-size files and a
# default line limit. Walks configured scan roots for *.rs files and fails
# if any file exceeds its effective limit.
#
# Env vars:
#   GATE_ROOT                 Repo root (default: git rev-parse --show-toplevel or pwd).
#   ALLOWLIST_PATH            Path to file-size-allowlist.json. Default resolves
#                             under GATE_ROOT at
#                             project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/baselines/file-size-allowlist.json
#   SCAN_ROOTS                Space-separated dirs to walk (default:
#                             "src crates/archon-tui/src").
#   ALLOWLIST_OVERRIDE_LINES  If "1", ratchet limit for allow-listed files is
#                             forced to 1 line (ratchet proof).

set -euo pipefail

# --- Resolve GATE_ROOT ---------------------------------------------------
if [[ -z "${GATE_ROOT:-}" ]]; then
    if GATE_ROOT=$(git rev-parse --show-toplevel 2>/dev/null); then
        :
    else
        GATE_ROOT="$(pwd)"
    fi
fi

DEFAULT_ALLOWLIST="project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/baselines/file-size-allowlist.json"
ALLOWLIST_PATH="${ALLOWLIST_PATH:-${GATE_ROOT}/${DEFAULT_ALLOWLIST}}"
SCAN_ROOTS="${SCAN_ROOTS:-src crates/archon-tui/src}"
ALLOWLIST_OVERRIDE_LINES="${ALLOWLIST_OVERRIDE_LINES:-0}"

# --- Preflight -----------------------------------------------------------
if ! command -v jq >/dev/null 2>&1; then
    echo "ERROR: jq is required but not installed" >&2
    exit 2
fi

# --- Load allow-list -----------------------------------------------------
MAX_LINES_DEFAULT=500
ALLOWLIST_JSON="{\"max_lines_default\":500,\"allowlist\":[]}"

if [[ -f "$ALLOWLIST_PATH" ]]; then
    if ! ALLOWLIST_JSON=$(jq -c '.' "$ALLOWLIST_PATH" 2>/dev/null); then
        echo "ERROR: allowlist JSON is malformed: $ALLOWLIST_PATH" >&2
        exit 2
    fi
fi

parsed_default=$(jq -r '.max_lines_default // 500' <<<"$ALLOWLIST_JSON")
if [[ "$parsed_default" =~ ^[0-9]+$ ]]; then
    MAX_LINES_DEFAULT="$parsed_default"
fi

# Build associative array: path -> current_lines
declare -A ALLOWLIST_LIMITS=()
while IFS=$'\t' read -r ap al; do
    [[ -z "$ap" ]] && continue
    ALLOWLIST_LIMITS["$ap"]="$al"
done < <(jq -r '.allowlist[]? | [.path, .current_lines] | @tsv' <<<"$ALLOWLIST_JSON")

# --- Walk scan roots -----------------------------------------------------
violations=()
files_checked=0
allowlisted_checked=0

for root in $SCAN_ROOTS; do
    abs_root="${GATE_ROOT%/}/${root}"
    [[ -d "$abs_root" ]] || continue

    while IFS= read -r -d '' abs_file; do
        rel_path="${abs_file#${GATE_ROOT%/}/}"
        lines=$(wc -l < "$abs_file" | tr -d ' ')
        files_checked=$((files_checked + 1))

        if [[ -n "${ALLOWLIST_LIMITS[$rel_path]:-}" ]]; then
            allowlisted_checked=$((allowlisted_checked + 1))
            if [[ "$ALLOWLIST_OVERRIDE_LINES" == "1" ]]; then
                effective_limit=1
            else
                effective_limit="${ALLOWLIST_LIMITS[$rel_path]}"
            fi
            if (( lines > effective_limit )); then
                violations+=("FAIL: $rel_path has $lines lines (limit $effective_limit, allowlist=yes)")
            fi
        else
            if (( lines > MAX_LINES_DEFAULT )); then
                violations+=("FAIL: $rel_path has $lines lines (limit $MAX_LINES_DEFAULT, allowlist=no)")
            fi
        fi
    done < <(find "$abs_root" -type f -name '*.rs' -print0)
done

# --- Report --------------------------------------------------------------
if (( ${#violations[@]} > 0 )); then
    for v in "${violations[@]}"; do
        echo "$v"
    done
    echo "FAILED: ${#violations[@]} violation(s) across $files_checked file(s) checked ($allowlisted_checked on allow-list)"
    exit 1
fi

echo "OK: $files_checked files checked, $allowlisted_checked on allow-list, 0 violations"
exit 0
