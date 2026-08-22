#!/usr/bin/env bash
# arch-lint — Enforces the D10 architectural rules from
# docs/architecture/spawn-everything-philosophy.md
#
# Created: TASK-AGS-100 (scaffold)
# Activated: TASK-AGS-110
# Re-pointed: the input handler left src/main.rs. Rules 1 and 3 were still
#             scoped to a marker region and a function-name list that had both
#             stopped matching anything, so they inspected zero lines and
#             reported success. See "Vacuity" below.
#
# Rules:
#   1 (D1, TC-ARCH-02): no .process_message().await anywhere in the interactive
#                       input-handling region
#   2 (D3, TC-ARCH-05): Agent event send must await bounded channel capacity
#   3 (D1 broad):       no .await on agent-turn work inside input handler
#                       functions
#
# Vacuity:
#   Every rule declares what it is about to scan and refuses to pass if that
#   target has gone missing: a region directory that no longer exists, a file
#   list that yields no candidate sites, an anchor that proves the region is
#   still the code the rule is about. A lint that cannot find its subject is
#   broken, not clean, so it exits non-zero saying so rather than warning and
#   continuing. Each rule also prints the counts it inspected, so the guarding
#   test can assert they are non-zero.
#
# Run locally:  bash scripts/lint/arch-lint.sh
# Run in CI:    via the `arch-lint` job in .github/workflows/ci.yml
#
# Exit codes:
#   0  clean
#   1  a forbidden pattern was found, or a rule could not find what it scans

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

# A rule whose scan target has vanished has stopped being a rule. Report that as
# a lint failure so the tree cannot go green on an inspection that never ran.
vacuous() {
    local rule="$1"
    local reason="$2"
    echo "arch-lint: RULE ${rule} HAS NOTHING TO SCAN — ${reason}" >&2
    echo "arch-lint: re-point the rule at where the code lives now; a rule that" >&2
    echo "           inspects nothing must never report success." >&2
    echo "see ${PHILOSOPHY_DOC}" >&2
    exit 1
}

# ---------------------------------------------------------------------------
# The interactive input-handling region.
#
# It used to be a BEGIN/END INPUT_HANDLER marker pair inside src/main.rs. That
# file is now a 113-line argument dispatcher: the session input loop lives in
# src/session_loop/ and the terminal key/mouse loop in
# crates/archon-tui/src/event_loop/. Two directories are a more durable region
# than two comment markers — a marker is deleted by whoever moves the code and
# takes the rule silently with it, whereas a directory that stops existing or
# stops dispatching turns is caught by the checks below.
# ---------------------------------------------------------------------------
INPUT_HANDLER_DIRS=(src/session_loop crates/archon-tui/src/event_loop)

# Populate REGION_FILES with every non-test source in the region.
REGION_FILES=()
collect_region_files() {
    local rule="$1"
    local dir
    for dir in "${INPUT_HANDLER_DIRS[@]}"; do
        if [[ ! -d "${dir}" ]]; then
            vacuous "${rule}" "input-handler region directory '${dir}' does not exist"
        fi
        local file
        while IFS= read -r file; do
            [[ -n "${file}" ]] && REGION_FILES+=("${file}")
        done < <(find "${dir}" -type f -name '*.rs' ! -name '*_tests.rs' | sort)
    done
    if [[ ${#REGION_FILES[@]} -eq 0 ]]; then
        vacuous "${rule}" "no .rs sources under ${INPUT_HANDLER_DIRS[*]}"
    fi
}

# Sum of `grep -c` counts across a file list. `grep -c` prints `path:count` for
# multiple files and exits 1 when nothing matched, which is not an error here.
count_matches() {
    local pattern="$1"
    shift
    grep -cE "${pattern}" "$@" 2>/dev/null | awk -F: '{ total += $NF } END { print total + 0 }'
}

# ---------------------------------------------------------------------------
# Rule 1 (TC-ARCH-02, D1): no .process_message().await in the input handler.
#
# This is the smoking gun from the philosophy doc: one synchronous
# `agent.process_message(&prompt).await` on the input path parks the whole event
# loop for the length of an agent turn. Agent work reaches the input path only
# as an `Arc<dyn TurnRunner>` handed to `AgentDispatcher::spawn_turn`, so the
# region has no legitimate reason to name `process_message` at all — not even
# inside a `tokio::spawn` block, because spawning agent work from the input
# handler is itself rule 3 of the philosophy. A flat ban inside the region is
# therefore both simpler and stricter than the old indentation heuristic, which
# cleared anything indented past 15 spaces.
#
# The pattern deliberately stops at the call and does not also demand `.await`
# on the same line. rustfmt breaks long method chains across lines, so
# `guard\n    .process_message(&prompt)\n    .await` defeats any single-line
# "call plus await" regex — and a `process_message` handle that the region
# merely holds is already outside the architecture, awaited or not.
# ---------------------------------------------------------------------------
RULE1_NAME='no .await on agent work in input handler (D1)'
collect_region_files 1

# Anchor: the region must still be the code that dispatches turns. If
# `spawn_turn` has left it, the input handler has moved again and this rule is
# guarding the wrong files — exactly the failure that let the marker-scoped
# version inspect nothing while reporting success.
RULE1_ANCHORS=$(count_matches '\.spawn_turn\(|fn spawn_turn' "${REGION_FILES[@]}")
if [[ "${RULE1_ANCHORS}" -eq 0 ]]; then
    vacuous 1 "no spawn_turn call under ${INPUT_HANDLER_DIRS[*]}; turn dispatch has moved out of the scanned region"
fi

# Candidate sites: every await in the region is something this rule had to look
# at and clear. Zero means the region is inert and the rule proves nothing.
RULE1_SITES=$(count_matches '\.await' "${REGION_FILES[@]}")
if [[ "${RULE1_SITES}" -eq 0 ]]; then
    vacuous 1 "no .await anywhere under ${INPUT_HANDLER_DIRS[*]}; nothing for the rule to clear"
fi

if match=$(grep -nE '\.process_message[[:space:]]*\(' "${REGION_FILES[@]}" 2>/dev/null); then
    fail "${RULE1_NAME}" "${match}"
fi

echo "arch-lint: rule=1 files=${#REGION_FILES[@]} sites=${RULE1_SITES} name=${RULE1_NAME}"

# ---------------------------------------------------------------------------
# Rule 2 (TC-ARCH-05, D3): Agent event send awaits bounded capacity.
#
# Lossless bounded transport requires the Agent producer to await capacity.
# ---------------------------------------------------------------------------
RULE2_NAME='Agent event transport must await bounded capacity (D3)'
AGENT_EVENTS_SOURCE="crates/archon-core/src/agent/events.rs"
if [[ ! -f "${AGENT_EVENTS_SOURCE}" ]]; then
    vacuous 2 "${AGENT_EVENTS_SOURCE} does not exist; the agent event transport has moved"
fi
RULE2_SITES=$(count_matches 'self\.event_tx\.send\(timestamped\)\.await' "${AGENT_EVENTS_SOURCE}")
if [[ "${RULE2_SITES}" -eq 0 ]]; then
    fail "${RULE2_NAME}" "${AGENT_EVENTS_SOURCE}: missing awaited TimestampedEvent send"
fi

echo "arch-lint: rule=2 files=1 sites=${RULE2_SITES} name=${RULE2_NAME}"

# ---------------------------------------------------------------------------
# Rule 3 (D1 broad): no .await on agent-turn work in input handler functions.
#
# The old version forbade *any* .await in these functions, which cannot be the
# rule: `handle_key_event` awaits a bounded `input_tx.send`, `dispatch_user_prompt`
# awaits `send_async` on the TUI channel, and the philosophy's own wording is
# "no .await >100ms". What a handler must never do is drive the agent turn
# itself — awaiting `process_message`/`run_turn` inline, or awaiting the
# JoinHandle of a previous turn, which serialises the loop just as effectively
# as the original smoking gun did.
# ---------------------------------------------------------------------------
RULE3_NAME='no .await on agent work in input handler function (D1 broad)'
RULE3_FN_PATTERN='fn[[:space:]]+(handle_[a-z0-9_]*|on_key|process_key|next_loop_input|dispatch_user_prompt|dispatch_terminal_event)[[:space:]]*[(<]'
RULE3_FORBIDDEN='\.(process_message|run_turn)\(.*\)\.await|(current_query|agent_task|turn_handle|join_handle)[^;]*\.await'

# rustfmt wraps long method chains, so the `.await` that makes a call blocking
# routinely sits on its own line several lines below the call. Rejoin every
# continuation line that starts with `.` onto the line before it, which is
# exactly the wrapping rustfmt introduced, before matching.
unwrap_method_chains() {
    sed -e ':a' -e 'N' -e '$!ba' -e 's/\n[[:space:]]*\./\./g'
}

RULE3_CANDIDATES=0
for file in "${REGION_FILES[@]}"; do
    fn_lines=$(grep -nE "${RULE3_FN_PATTERN}" "${file}" 2>/dev/null | cut -d: -f1)
    for fn_line in ${fn_lines}; do
        RULE3_CANDIDATES=$((RULE3_CANDIDATES + 1))
        # Heuristic window: ~200 lines from the function header. The forbidden
        # patterns are narrow enough that overshooting into the next function
        # still only flags real awaited agent work.
        chunk=$(sed -n "${fn_line},$((fn_line + 200))p" "${file}" | unwrap_method_chains)
        if echo "${chunk}" | grep -qE "${RULE3_FORBIDDEN}"; then
            match=$(echo "${chunk}" | grep -E "${RULE3_FORBIDDEN}" | head -3)
            fail "${RULE3_NAME}" "${file}:${fn_line}+: ${match}"
        fi
    done
done

if [[ "${RULE3_CANDIDATES}" -eq 0 ]]; then
    vacuous 3 "no function matching the input-handler naming convention under ${INPUT_HANDLER_DIRS[*]}; the convention changed and the rule stopped matching anything"
fi

echo "arch-lint: rule=3 files=${#REGION_FILES[@]} sites=${RULE3_CANDIDATES} name=${RULE3_NAME}"

echo "arch-lint: all checks passed"
exit 0
