#!/usr/bin/env bash
set -euo pipefail

cd /home/unixdude/Archon-projects/archon-cli-worktrees/archonfixes

SCRIPT_PATH="scripts/tui-baseline-loc.sh"
BASELINE_DIR="project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/baselines"
LOC_JSON="${BASELINE_DIR}/loc-baseline.json"
LOC_MD="${BASELINE_DIR}/loc-baseline.md"
ALLOWLIST_JSON="${BASELINE_DIR}/file-size-allowlist.json"

# 1. Verify the baseline generator script exists and is executable
if [ ! -f "${SCRIPT_PATH}" ]; then
  echo "FAIL: ${SCRIPT_PATH} does not exist"
  exit 1
fi
if [ ! -x "${SCRIPT_PATH}" ]; then
  echo "FAIL: ${SCRIPT_PATH} is not executable"
  exit 1
fi

# 2. Verify baseline artefacts exist
if [ ! -f "${LOC_JSON}" ]; then
  echo "FAIL: ${LOC_JSON} does not exist"
  exit 1
fi
if [ ! -f "${LOC_MD}" ]; then
  echo "FAIL: ${LOC_MD} does not exist"
  exit 1
fi
if [ ! -f "${ALLOWLIST_JSON}" ]; then
  echo "FAIL: ${ALLOWLIST_JSON} does not exist"
  exit 1
fi

# 3. jq assertions on loc-baseline.json
if ! command -v jq >/dev/null 2>&1; then
  echo "FAIL: jq is not installed on PATH"
  exit 1
fi

if ! jq -e '.files | length > 0' "${LOC_JSON}" >/dev/null; then
  echo "FAIL: ${LOC_JSON} .files is empty or missing"
  exit 1
fi

if ! jq -e '.over_500 | length >= 1' "${LOC_JSON}" >/dev/null; then
  echo "FAIL: ${LOC_JSON} .over_500 must have at least 1 entry"
  exit 1
fi

if ! jq -e '.files == (.files | sort_by(-.lines))' "${LOC_JSON}" >/dev/null; then
  echo "FAIL: ${LOC_JSON} .files is not sorted descending by .lines"
  exit 1
fi

if ! jq -e '[.files[] | select(.lines >= 5000)] | length > 0' "${LOC_JSON}" >/dev/null; then
  echo "FAIL: ${LOC_JSON} has no file with lines >= 5000 (expected src/main.rs ~6158)"
  exit 1
fi

# 4. jq assertions on file-size-allowlist.json
if ! jq -e '.allowlist | length >= 2' "${ALLOWLIST_JSON}" >/dev/null; then
  echo "FAIL: ${ALLOWLIST_JSON} .allowlist must have at least 2 entries"
  exit 1
fi

if ! jq -e '.allowlist | map(.path) | index("src/main.rs") != null' "${ALLOWLIST_JSON}" >/dev/null; then
  echo "FAIL: ${ALLOWLIST_JSON} .allowlist is missing path 'src/main.rs'"
  exit 1
fi

if ! jq -e '.allowlist | map(.path) | index("crates/archon-tui/src/app.rs") != null' "${ALLOWLIST_JSON}" >/dev/null; then
  echo "FAIL: ${ALLOWLIST_JSON} .allowlist is missing path 'crates/archon-tui/src/app.rs'"
  exit 1
fi

if ! jq -e '[.allowlist[].target_phase | test("^phase-[1-9]$")] | all' "${ALLOWLIST_JSON}" >/dev/null; then
  echo "FAIL: ${ALLOWLIST_JSON} has a .target_phase that does not match ^phase-[1-9]$"
  exit 1
fi

# 5. Markdown table header check
if [ "$(grep -c -E 'path.*lines.*target_phase.*target_lines' "${LOC_MD}" || true)" -lt 1 ]; then
  echo "FAIL: ${LOC_MD} is missing a header line containing path, lines, target_phase, target_lines"
  exit 1
fi

echo "OK: verify-TUI-002 passed"
exit 0
