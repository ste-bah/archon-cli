#!/usr/bin/env bash
# Reference: project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/TASK-TUI-008.md
set -euo pipefail

cd /home/unixdude/Archon-projects/archon-cli-worktrees/archonfixes

CRATE_DIR="crates/archon-tui-test-support"
CARGO_TOML="${CRATE_DIR}/Cargo.toml"
MOCK_AGENT="${CRATE_DIR}/src/mock_agent.rs"
TEST_FILE="${CRATE_DIR}/tests/mock_agent_smoke.rs"

# ---- 1. mock_agent.rs exists --------------------------------------------
if [[ ! -f "${MOCK_AGENT}" ]]; then
  echo "FAIL: missing ${MOCK_AGENT}" >&2
  exit 1
fi

# ---- 2. contains pub struct MockAgent -----------------------------------
if ! grep -qE 'pub struct MockAgent\b' "${MOCK_AGENT}"; then
  echo "FAIL: ${MOCK_AGENT} missing pub struct MockAgent" >&2
  exit 1
fi

# ---- 3. contains pub struct EventScript ---------------------------------
if ! grep -qE 'pub struct EventScript\b' "${MOCK_AGENT}"; then
  echo "FAIL: ${MOCK_AGENT} missing pub struct EventScript" >&2
  exit 1
fi

# ---- 4. contains pub enum MockEventKind ---------------------------------
if ! grep -qE 'pub enum MockEventKind\b' "${MOCK_AGENT}"; then
  echo "FAIL: ${MOCK_AGENT} missing pub enum MockEventKind" >&2
  exit 1
fi

# ---- 5. all 5 MockEventKind variants ------------------------------------
for variant in ToolCall MessageDelta ThoughtChunk ToolResult Finish; do
  if ! grep -qE "\b${variant}\b" "${MOCK_AGENT}"; then
    echo "FAIL: ${MOCK_AGENT} missing MockEventKind variant ${variant}" >&2
    exit 1
  fi
done

# ---- 6. contains pub trait MockEventSink --------------------------------
if ! grep -qE 'pub trait MockEventSink\b' "${MOCK_AGENT}"; then
  echo "FAIL: ${MOCK_AGENT} missing pub trait MockEventSink" >&2
  exit 1
fi

# ---- 7. contains pub struct MockAgentReport -----------------------------
if ! grep -qE 'pub struct MockAgentReport\b' "${MOCK_AGENT}"; then
  echo "FAIL: ${MOCK_AGENT} missing pub struct MockAgentReport" >&2
  exit 1
fi

# ---- 8. MockAgentReport has all 4 fields --------------------------------
for field in events_sent events_dropped elapsed cancelled; do
  if ! grep -qE "\b${field}\b" "${MOCK_AGENT}"; then
    echo "FAIL: ${MOCK_AGENT} missing MockAgentReport field ${field}" >&2
    exit 1
  fi
done

# ---- 9. contains pub fn spawn_n_mock_agents -----------------------------
if ! grep -qE 'pub fn spawn_n_mock_agents\b' "${MOCK_AGENT}"; then
  echo "FAIL: ${MOCK_AGENT} missing pub fn spawn_n_mock_agents" >&2
  exit 1
fi

# ---- 10. EventScript builder methods ------------------------------------
for method in tool_call message_delta burst_of sleep; do
  if ! grep -qE "pub fn ${method}\s*(<|\()" "${MOCK_AGENT}"; then
    echo "FAIL: ${MOCK_AGENT} missing pub fn ${method} on EventScript" >&2
    exit 1
  fi
done

# ---- 11. MockAgent methods ----------------------------------------------
if ! grep -qE 'pub fn new\s*(<|\()' "${MOCK_AGENT}"; then
  echo "FAIL: ${MOCK_AGENT} missing pub fn new on MockAgent" >&2
  exit 1
fi
if ! grep -qE 'pub async fn run\s*(<|\()' "${MOCK_AGENT}"; then
  echo "FAIL: ${MOCK_AGENT} missing pub async fn run on MockAgent" >&2
  exit 1
fi
if ! grep -qE 'pub fn cancel\s*(<|\()' "${MOCK_AGENT}"; then
  echo "FAIL: ${MOCK_AGENT} missing pub fn cancel on MockAgent" >&2
  exit 1
fi

# ---- 12. Cargo.toml has tokio-util 0.7 ----------------------------------
if [[ ! -f "${CARGO_TOML}" ]]; then
  echo "FAIL: missing ${CARGO_TOML}" >&2
  exit 1
fi
if ! grep -qE 'tokio-util\s*=' "${CARGO_TOML}"; then
  echo "FAIL: ${CARGO_TOML} missing tokio-util dep" >&2
  exit 1
fi
if ! grep -E 'tokio-util\s*=' "${CARGO_TOML}" | grep -q '0\.7'; then
  echo "FAIL: ${CARGO_TOML} tokio-util not version 0.7" >&2
  exit 1
fi

# ---- 13. tokio-util line mentions "rt" feature --------------------------
if ! grep -E 'tokio-util\s*=' "${CARGO_TOML}" | grep -q '"rt"'; then
  echo "FAIL: ${CARGO_TOML} tokio-util missing \"rt\" feature" >&2
  exit 1
fi

# ---- 14. mock_agent_smoke.rs exists -------------------------------------
if [[ ! -f "${TEST_FILE}" ]]; then
  echo "FAIL: missing ${TEST_FILE}" >&2
  exit 1
fi

# ---- 15. exactly 5 #[test] attributes -----------------------------------
TEST_COUNT=$(grep -c '^#\[test\]' "${TEST_FILE}" || true)
if [[ "${TEST_COUNT}" != "5" ]]; then
  echo "FAIL: ${TEST_FILE} expected 5 #[test] attributes, found ${TEST_COUNT}" >&2
  exit 1
fi

# ---- 16. contains fn scripted_events_are_emitted_in_order ---------------
if ! grep -qE 'fn scripted_events_are_emitted_in_order\b' "${TEST_FILE}"; then
  echo "FAIL: ${TEST_FILE} missing fn scripted_events_are_emitted_in_order" >&2
  exit 1
fi

# ---- 17. contains fn cancel_aborts_run_within_100ms_wallclock_with_paused_time
if ! grep -qE 'fn cancel_aborts_run_within_100ms_wallclock_with_paused_time\b' "${TEST_FILE}"; then
  echo "FAIL: ${TEST_FILE} missing fn cancel_aborts_run_within_100ms_wallclock_with_paused_time" >&2
  exit 1
fi

# ---- 18. contains fn spawn_100_mock_agents_each_10_events_all_complete --
if ! grep -qE 'fn spawn_100_mock_agents_each_10_events_all_complete\b' "${TEST_FILE}"; then
  echo "FAIL: ${TEST_FILE} missing fn spawn_100_mock_agents_each_10_events_all_complete" >&2
  exit 1
fi

# ---- 19. contains fn unbounded_sink_never_drops -------------------------
if ! grep -qE 'fn unbounded_sink_never_drops\b' "${TEST_FILE}"; then
  echo "FAIL: ${TEST_FILE} missing fn unbounded_sink_never_drops" >&2
  exit 1
fi

# ---- 20. contains fn bounded_sink_reports_drops_on_full -----------------
if ! grep -qE 'fn bounded_sink_reports_drops_on_full\b' "${TEST_FILE}"; then
  echo "FAIL: ${TEST_FILE} missing fn bounded_sink_reports_drops_on_full" >&2
  exit 1
fi

# ---- 21. cargo test mock_agent_smoke passes -----------------------------
set +e
TEST_OUT=$(cargo test -p archon-tui-test-support --test mock_agent_smoke -j1 -- --test-threads=2 2>&1 | tail -30)
TEST_EXIT=$?
set -e
echo "${TEST_OUT}"
if [[ "${TEST_EXIT}" != "0" ]]; then
  echo "FAIL: cargo test mock_agent_smoke exited ${TEST_EXIT}" >&2
  exit 1
fi

# ---- 22. workspace still type-checks ------------------------------------
set +e
CHECK_OUT=$(cargo check --workspace -j1 2>&1 | tail -10)
CHECK_EXIT=$?
set -e
echo "${CHECK_OUT}"
if [[ "${CHECK_EXIT}" != "0" ]]; then
  echo "FAIL: cargo check --workspace exited ${CHECK_EXIT}" >&2
  exit 1
fi

# ---- 23. no scope creep into src/main.rs or crates/archon-tui/src/ ------
SCOPE_CREEP=$(git diff --name-only src/main.rs crates/archon-tui/src/ 2>/dev/null | wc -l | tr -d ' ')
if [[ "${SCOPE_CREEP}" != "0" ]]; then
  echo "FAIL: src/main.rs or crates/archon-tui/src/ was modified (out of scope for TUI-008)" >&2
  git diff --name-only src/main.rs crates/archon-tui/src/ 2>/dev/null >&2 || true
  exit 1
fi

echo "OK: verify-TUI-008 passed"
