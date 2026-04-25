#!/usr/bin/env bash
# arch-lint — Enforces the D10 architectural rules from
# docs/architecture/spawn-everything-philosophy.md
#
# Created: TASK-AGS-100 (scaffold)
# Activated: TASK-AGS-110
#
# Rules:
#   1 (D1, TC-ARCH-02): no .process_message().await at handler scope in INPUT_HANDLER region
#   2 (D3, TC-ARCH-05): no .send().await on agent event channel (must be unbounded)
#   3 (D1 broad):       no .await in functions named handle_*_input / on_key / process_key
#
# Run locally:  bash scripts/lint/arch-lint.sh
# Run in CI:    via the `arch-lint` job in .github/workflows/ci.yml
#
# Exit codes:
#   0  clean
#   1  a forbidden pattern was found

set -u
set -o pipefail

# Resolve repo root from this script's location.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
cd "${REPO_ROOT}"

PHILOSOPHY_DOC="docs/architecture/spawn-everything-philosophy.md"

fail() {
    local rule="$1"
    local match="$2"
    echo "arch-lint: FORBIDDEN pattern for rule '${rule}':" >&2
    echo "${match}" >&2
    echo "see ${PHILOSOPHY_DOC}" >&2
    exit 1
}

# ---------------------------------------------------------------------------
# Rule 1 (TC-ARCH-02, D1): no .process_message().await at handler scope
#
# Scoped to lines between BEGIN INPUT_HANDLER and END INPUT_HANDLER markers
# in src/main.rs. Only flags calls at handler body indentation (<=15 spaces),
# NOT calls inside tokio::spawn blocks (16+ spaces). The spawn pattern is
# correct — it's the direct .await that blocks the handler.
# ---------------------------------------------------------------------------
BEGIN_LINE=$(grep -n 'BEGIN INPUT_HANDLER' src/main.rs 2>/dev/null | head -1 | cut -d: -f1)
END_LINE=$(grep -n 'END INPUT_HANDLER' src/main.rs 2>/dev/null | head -1 | cut -d: -f1)

if [[ -n "${BEGIN_LINE}" && -n "${END_LINE}" ]]; then
    # Match .process_message().await only at <=15 leading spaces (handler scope)
    # Lines at 16+ spaces are inside tokio::spawn blocks (correct pattern)
    if match=$(sed -n "${BEGIN_LINE},${END_LINE}p" src/main.rs \
        | grep -nE '^[[:space:]]{0,15}[^[:space:]].*\.process_message\([^)]*\)\.await' 2>/dev/null); then
        fail "no .await on agent work in input handler (D1)" "src/main.rs INPUT_HANDLER region: ${match}"
    fi
else
    echo "arch-lint: WARNING — BEGIN/END INPUT_HANDLER markers not found in src/main.rs" >&2
    # No markers = can't scope; warn but don't fail (markers might be in transit)
fi

# ---------------------------------------------------------------------------
# Rule 2 (TC-ARCH-05, D3): no .send().await on agent event channel
#
# The agent event channel MUST be unbounded (non-async send). Any
# `event_tx.send(...).await` in agent.rs or main.rs is a violation.
# ---------------------------------------------------------------------------
RULE2_PATTERN='event_tx\.send\([^)]*\)\.await'
RULE2_PATHS=(crates/archon-core/src/agent.rs src/main.rs)

if match=$(grep -nE "${RULE2_PATTERN}" "${RULE2_PATHS[@]}" 2>/dev/null); then
    fail "producer channels must be unbounded — no .send().await on event_tx (D3)" "${match}"
fi

# ---------------------------------------------------------------------------
# Rule 3 (D1 broad): no .await in input handler functions
#
# Heuristic fallback: catches any .await inside functions whose names
# suggest they handle user input directly. These functions must delegate
# async work via tokio::spawn, not .await directly.
# ---------------------------------------------------------------------------
RULE3_FN_PATTERN='fn[[:space:]]+(handle_.*_input|on_key|process_key)[[:space:]]*\('
RULE3_PATHS=(src/main.rs crates/archon-tui/src/app.rs)

for file in "${RULE3_PATHS[@]}"; do
    if [[ ! -f "${file}" ]]; then
        continue
    fi
    fn_lines=$(grep -nE "${RULE3_FN_PATTERN}" "${file}" 2>/dev/null | cut -d: -f1)
    for fn_line in $fn_lines; do
        # Extract ~200 lines from function start and look for .await
        chunk=$(sed -n "${fn_line},$((fn_line + 200))p" "${file}")
        if echo "${chunk}" | grep -qE '\.await'; then
            match=$(echo "${chunk}" | grep -nE '\.await' | head -3)
            fail "no .await in input handler function (D1 broad)" "${file}:${fn_line}+: ${match}"
        fi
    done
done

echo "arch-lint: all checks passed"
exit 0
