#!/usr/bin/env bash
#
# tui-baseline-loc.sh
#
# Capture a line-of-code baseline of the Rust source tree for the TUI
# refactor (TASK-TUI-002). Produces three artefacts under
# project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/baselines/:
#
#   1. loc-baseline.json       - machine readable, sorted desc
#   2. loc-baseline.md         - human readable markdown
#   3. file-size-allowlist.json - allow-list consumed by TASK-TUI-003
#
# Walks src/**/*.rs and crates/archon-tui/src/**/*.rs.

set -euo pipefail

cd "$(dirname "$0")/.."

BASELINE_DIR="project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/baselines"
LOC_JSON="${BASELINE_DIR}/loc-baseline.json"
LOC_MD="${BASELINE_DIR}/loc-baseline.md"
ALLOWLIST_JSON="${BASELINE_DIR}/file-size-allowlist.json"

mkdir -p "${BASELINE_DIR}"

TS="$(date -Iseconds)"
SHA="$(git rev-parse HEAD)"

# Collect { path, lines } for every .rs file under the two roots.
# Use a temp file of newline-delimited JSON objects, then slurp with jq.
TMP_NDJSON="$(mktemp)"
trap 'rm -f "${TMP_NDJSON}"' EXIT

emit_file() {
  local f="$1"
  local lines
  lines="$(wc -l < "$f" | tr -d '[:space:]')"
  jq -n --arg path "$f" --argjson lines "$lines" '{path: $path, lines: $lines}'
}

while IFS= read -r f; do
  emit_file "$f" >> "${TMP_NDJSON}"
done < <(find src -name '*.rs' -type f)

while IFS= read -r f; do
  emit_file "$f" >> "${TMP_NDJSON}"
done < <(find crates/archon-tui/src -name '*.rs' -type f)

# Sorted descending list of all files.
FILES_JSON="$(jq -s 'sort_by(-.lines)' "${TMP_NDJSON}")"
# Subset over 500 lines.
OVER500_JSON="$(echo "${FILES_JSON}" | jq '[ .[] | select(.lines > 500) ]')"

# 1. loc-baseline.json
jq -n \
  --arg ts "$TS" \
  --arg sha "$SHA" \
  --argjson files "$FILES_JSON" \
  --argjson over500 "$OVER500_JSON" \
  '{generated_at: $ts, git_commit: $sha, files: $files, over_500: $over500}' \
  > "${LOC_JSON}"

# 2. file-size-allowlist.json
#    Every file with > 500 lines becomes an allowlist entry with
#    target_phase = phase-3 and target_lines = 500.
ALLOWLIST_ENTRIES="$(echo "${OVER500_JSON}" | jq '
  [ .[] | {path: .path, current_lines: .lines, target_phase: "phase-3", target_lines: 500} ]
')"

jq -n \
  --argjson allowlist "$ALLOWLIST_ENTRIES" \
  '{max_lines_default: 500, allowlist: $allowlist}' \
  > "${ALLOWLIST_JSON}"

# 3. loc-baseline.md
{
  echo "# LoC Baseline -- TUI Refactor"
  echo
  echo "This document is the day-0 line-of-count baseline captured by"
  echo "TASK-TUI-002 on ${TS} at git commit \`${SHA}\`. It enumerates every"
  echo "Rust source file under \`src/\` and \`crates/archon-tui/src/\` and"
  echo "flags the files that exceed the 500-line budget. Files listed in"
  echo "the table below are the refactor targets for phase-3; the"
  echo "downstream TASK-TUI-003 file-size CI gate consumes the companion"
  echo "\`file-size-allowlist.json\` so these oversized files are"
  echo "temporarily tolerated until they are split."
  echo
  echo "| path | lines | target_phase | target_lines |"
  echo "| --- | ---: | --- | ---: |"
  echo "${OVER500_JSON}" | jq -r '.[] | "| \(.path) | \(.lines) | phase-3 | 500 |"'
  echo
  echo "## All source files (ranked)"
  echo
  echo "${FILES_JSON}" | jq -r '.[] | "- \(.lines) \(.path)"'
} > "${LOC_MD}"

TOTAL="$(jq '.files | length' "${LOC_JSON}")"
OVER="$(jq '.over_500 | length' "${LOC_JSON}")"
echo "Baseline captured: ${TOTAL} files total, ${OVER} over 500 lines"
