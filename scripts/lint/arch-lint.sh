#!/usr/bin/env bash
# arch-lint — TASK-AGS-100 scaffold, activated by TASK-AGS-110.
#
# Enforces the three D10 rules from
# docs/architecture/spawn-everything-philosophy.md:
#
#   1. no .await >100ms in main event handler
#   2. producer channels are unbounded
#   3. tools own task lifecycle
#
# Until TASK-AGS-110 lands, the activation lines below are commented out and
# this script exits 0. TASK-AGS-110 will uncomment the `grep` invocations and
# turn the script into an enforcing check.
#
# Run locally:  bash scripts/lint/arch-lint.sh
# Run in CI:    via the `arch-lint` job in .github/workflows/ci.yml
#
# Exit codes:
#   0  success (or scaffold mode)
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
# Rule 1 — no .await on agent work in the main input handler.
# Forbidden pattern: `agent.process_message(...).await` or
#                    `agent\.process_message\(.*\)\.await`
# Activation deferred to TASK-AGS-110.
# ---------------------------------------------------------------------------
RULE1_PATTERN='agent\.process_message\([^)]*\)\.await'
RULE1_PATHS=(src/main.rs)
# TASK-AGS-110: uncomment the block below to activate.
# if match=$(grep -nE "${RULE1_PATTERN}" "${RULE1_PATHS[@]}" 2>/dev/null); then
#     fail "no .await >100ms in main event handler" "${match}"
# fi

# ---------------------------------------------------------------------------
# Rule 2 — producer channels are unbounded.
# Forbidden pattern: `mpsc::channel::<AgentEvent>(` (bounded constructor on
# the agent-event carrier).
# Activation deferred to TASK-AGS-110.
# ---------------------------------------------------------------------------
RULE2_PATTERN='mpsc::channel::<AgentEvent>\('
RULE2_PATHS=(src/main.rs)
# TASK-AGS-110: uncomment the block below to activate.
# if match=$(grep -nE "${RULE2_PATTERN}" "${RULE2_PATHS[@]}" 2>/dev/null); then
#     fail "producer channels are unbounded" "${match}"
# fi

# ---------------------------------------------------------------------------
# Rule 3 — tools own task lifecycle.
# Forbidden pattern: direct `tokio::spawn` inside the agent loop for subagent
# work (covered by TC-ARCH-05). Paths are restricted so grep does not trigger
# on legitimate spawn sites inside tool implementations or the registry.
# Activation deferred to TASK-AGS-110.
# ---------------------------------------------------------------------------
RULE3_PATTERN='tokio::spawn\('
RULE3_PATHS=(crates/archon-core/src/agent.rs)
# TASK-AGS-110: uncomment the block below to activate (note: TASK-AGS-105
# removes the legacy spawn at agent.rs:2939-2977 first).
# if match=$(grep -nE "${RULE3_PATTERN}" "${RULE3_PATHS[@]}" 2>/dev/null); then
#     fail "tools own task lifecycle" "${match}"
# fi

echo "arch-lint: scaffold mode — no patterns active (activation in TASK-AGS-110)"
exit 0
