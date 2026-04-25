#!/usr/bin/env bash
# Reference: project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/TASK-TUI-011.md
#
# Gate 1 static-assertion verifier for TASK-TUI-011 (phase-0 closeout & handoff).
#
# This script enforces the acceptance criteria of TASK-TUI-011 statically
# (file existence, grep, jq shape checks) WITHOUT running the full capture
# (running the capture is the Gate 5 live-smoke target).
#
# 22 numbered assertions, strict bash, any failure -> exit 1 with FAIL message.

set -euo pipefail

REPO="/home/unixdude/Archon-projects/archon-cli-worktrees/archonfixes"
cd "$REPO"

fail() { echo "FAIL: $*" >&2; exit 1; }

CAPTURE_SCRIPT="scripts/tui-phase0-capture.sh"
BASELINES_DIR="project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/baselines"
MANIFEST="${BASELINES_DIR}/phase0-capture-manifest.json"
HANDOFF="project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/HANDOFF.md"
INDEX="project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/_index.md"
LOC_BASELINE="${BASELINES_DIR}/loc-baseline.json"
BENCH_BASELINE="${BASELINES_DIR}/bench-eventloop-baseline.json"

# ---- 1. scripts/tui-phase0-capture.sh exists ----------------------------
echo "ASSERT 1: ${CAPTURE_SCRIPT} exists"
[ -f "${CAPTURE_SCRIPT}" ] || fail "${CAPTURE_SCRIPT} missing"

# ---- 2. scripts/tui-phase0-capture.sh is executable ---------------------
echo "ASSERT 2: ${CAPTURE_SCRIPT} is executable"
[ -x "${CAPTURE_SCRIPT}" ] || fail "${CAPTURE_SCRIPT} is not executable"

# ---- 3. shebang is bash -------------------------------------------------
echo "ASSERT 3: ${CAPTURE_SCRIPT} has bash shebang"
FIRST_LINE=$(head -n 1 "${CAPTURE_SCRIPT}")
if [[ "${FIRST_LINE}" != "#!/usr/bin/env bash" && "${FIRST_LINE}" != "#!/bin/bash" ]]; then
  fail "${CAPTURE_SCRIPT} first line must be '#!/usr/bin/env bash' or '#!/bin/bash' (got: ${FIRST_LINE})"
fi

# ---- 4. contains set -euo pipefail --------------------------------------
echo "ASSERT 4: ${CAPTURE_SCRIPT} contains 'set -euo pipefail'"
grep -qF 'set -euo pipefail' "${CAPTURE_SCRIPT}" || fail "${CAPTURE_SCRIPT} missing 'set -euo pipefail'"

# ---- 5. invokes bash scripts/tui-baseline-loc.sh ------------------------
echo "ASSERT 5: ${CAPTURE_SCRIPT} invokes 'bash scripts/tui-baseline-loc.sh'"
grep -qF 'bash scripts/tui-baseline-loc.sh' "${CAPTURE_SCRIPT}" \
  || fail "${CAPTURE_SCRIPT} missing 'bash scripts/tui-baseline-loc.sh' invocation"

# ---- 6. invokes bash scripts/tui-file-size-gate.sh ----------------------
echo "ASSERT 6: ${CAPTURE_SCRIPT} invokes 'bash scripts/tui-file-size-gate.sh'"
grep -qF 'bash scripts/tui-file-size-gate.sh' "${CAPTURE_SCRIPT}" \
  || fail "${CAPTURE_SCRIPT} missing 'bash scripts/tui-file-size-gate.sh' invocation"

# ---- 7. invokes bash scripts/tui-banned-patterns-gate.sh ----------------
echo "ASSERT 7: ${CAPTURE_SCRIPT} invokes 'bash scripts/tui-banned-patterns-gate.sh'"
grep -qF 'bash scripts/tui-banned-patterns-gate.sh' "${CAPTURE_SCRIPT}" \
  || fail "${CAPTURE_SCRIPT} missing 'bash scripts/tui-banned-patterns-gate.sh' invocation"

# ---- 8. invokes cargo test -p archon-tui-test-support -------------------
echo "ASSERT 8: ${CAPTURE_SCRIPT} invokes 'cargo test -p archon-tui-test-support'"
grep -qF 'cargo test -p archon-tui-test-support' "${CAPTURE_SCRIPT}" \
  || fail "${CAPTURE_SCRIPT} missing 'cargo test -p archon-tui-test-support' invocation"

# ---- 9. invokes cargo bench eventloop_throughput ------------------------
echo "ASSERT 9: ${CAPTURE_SCRIPT} invokes 'cargo bench -p archon-tui-test-support --bench eventloop_throughput'"
grep -qF 'cargo bench -p archon-tui-test-support --bench eventloop_throughput' "${CAPTURE_SCRIPT}" \
  || fail "${CAPTURE_SCRIPT} missing 'cargo bench -p archon-tui-test-support --bench eventloop_throughput' invocation"

# ---- 10. writes to the manifest path ------------------------------------
echo "ASSERT 10: ${CAPTURE_SCRIPT} writes to ${MANIFEST}"
grep -qF 'project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/baselines/phase0-capture-manifest.json' "${CAPTURE_SCRIPT}" \
  || fail "${CAPTURE_SCRIPT} does not reference ${MANIFEST}"

# ---- 11. manifest exists and is valid JSON ------------------------------
echo "ASSERT 11: ${MANIFEST} exists and parses as JSON"
[ -f "${MANIFEST}" ] || fail "${MANIFEST} missing"
jq empty "${MANIFEST}" >/dev/null 2>&1 || fail "${MANIFEST} is not valid JSON"

# ---- 12. manifest has required top-level keys ---------------------------
echo "ASSERT 12: ${MANIFEST} has all required keys"
for key in captured_at git_commit loc_baseline_sha file_size_gate banned_patterns_gate cargo_test_archon_tui_test_support bench_eventloop_baseline_sha artefacts; do
  jq -e ". | has(\"${key}\")" "${MANIFEST}" >/dev/null 2>&1 \
    || fail "${MANIFEST} missing required key '${key}'"
done

# ---- 13. gate statuses all == "pass" ------------------------------------
echo "ASSERT 13: ${MANIFEST} file_size_gate/banned_patterns_gate/cargo_test_archon_tui_test_support all == 'pass'"
FSG=$(jq -r '.file_size_gate' "${MANIFEST}")
BPG=$(jq -r '.banned_patterns_gate' "${MANIFEST}")
CTS=$(jq -r '.cargo_test_archon_tui_test_support' "${MANIFEST}")
[ "${FSG}" = "pass" ] || fail "${MANIFEST} file_size_gate != 'pass' (got: ${FSG})"
[ "${BPG}" = "pass" ] || fail "${MANIFEST} banned_patterns_gate != 'pass' (got: ${BPG})"
[ "${CTS}" = "pass" ] || fail "${MANIFEST} cargo_test_archon_tui_test_support != 'pass' (got: ${CTS})"

# ---- 14. manifest.artefacts is a non-empty JSON array -------------------
echo "ASSERT 14: ${MANIFEST} artefacts is a non-empty array"
jq -e '.artefacts | type == "array" and length > 0' "${MANIFEST}" >/dev/null 2>&1 \
  || fail "${MANIFEST} artefacts is not a non-empty array"

# ---- 15. HANDOFF.md exists ----------------------------------------------
echo "ASSERT 15: ${HANDOFF} exists"
[ -f "${HANDOFF}" ] || fail "${HANDOFF} missing"

# ---- 16. HANDOFF.md contains exactly 9 '## Phase-' headings -------------
echo "ASSERT 16: ${HANDOFF} has exactly 9 lines matching '^## Phase-'"
PHASE_COUNT=$(grep -c '^## Phase-' "${HANDOFF}" || true)
if [ "${PHASE_COUNT}" != "9" ]; then
  fail "${HANDOFF} expected 9 '## Phase-' headings, found ${PHASE_COUNT}"
fi

# ---- 17. HANDOFF.md contains non-modification attestation ---------------
echo "ASSERT 17: ${HANDOFF} contains 'Phase 0 did not modify any production source file'"
grep -qF 'Phase 0 did not modify any production source file' "${HANDOFF}" \
  || fail "${HANDOFF} missing 'Phase 0 did not modify any production source file' attestation"

# ---- 18. HANDOFF.md mentions each sibling TUI task ----------------------
echo "ASSERT 18: ${HANDOFF} mentions TUI-002..TUI-004, TUI-006..TUI-010"
for id in TASK-TUI-002 TASK-TUI-003 TASK-TUI-004 TASK-TUI-006 TASK-TUI-007 TASK-TUI-008 TASK-TUI-009 TASK-TUI-010; do
  grep -qF "${id}" "${HANDOFF}" || fail "${HANDOFF} missing mention of ${id}"
done

# ---- 19. _index.md lists TUI-011 with status 'completed' ----------------
echo "ASSERT 19: ${INDEX} mentions TASK-TUI-011 with status 'completed'"
[ -f "${INDEX}" ] || fail "${INDEX} missing"
if ! awk '/TASK-TUI-011/ && /completed/ { found = 1 } END { exit !found }' "${INDEX}"; then
  fail "${INDEX} does not mention TASK-TUI-011 with status 'completed'"
fi

# ---- 20. no scope creep into production source -------------------------
echo "ASSERT 20: src/main.rs, crates/archon-tui/src/, crates/archon-tools/ untouched"
SCOPE_CREEP=$(git diff --name-only src/main.rs crates/archon-tui/src/ crates/archon-tools/ 2>/dev/null | wc -l | tr -d ' ')
if [ "${SCOPE_CREEP}" != "0" ]; then
  git diff --name-only src/main.rs crates/archon-tui/src/ crates/archon-tools/ 2>/dev/null >&2 || true
  fail "src/main.rs, crates/archon-tui/src/, or crates/archon-tools/ was modified (out of scope for TUI-011)"
fi

# ---- 21. loc-baseline.json exists ---------------------------------------
echo "ASSERT 21: ${LOC_BASELINE} exists"
[ -f "${LOC_BASELINE}" ] || fail "${LOC_BASELINE} missing"

# ---- 22. bench-eventloop-baseline.json exists ---------------------------
echo "ASSERT 22: ${BENCH_BASELINE} exists"
[ -f "${BENCH_BASELINE}" ] || fail "${BENCH_BASELINE} missing"

echo "OK: verify-TUI-011 passed"
