#!/usr/bin/env bash
# Reference: project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/TASK-TUI-010.md
#
# Gate 1 static-assertion verifier for TASK-TUI-010.
#
# This script enforces the acceptance criteria of TASK-TUI-010 statically
# (file existence, grep) without running cargo. It is intentionally strict:
# every assertion is numbered and emits a clear FAIL message.
#
# Expected test count in tests/fake_registry_smoke.rs is 5:
#   1. insert_then_poll_returns_done_after_task_completes
#   2. poll_unknown_id_returns_notfound
#   3. concurrent_100_inserts_all_under_10ms
#   4. await_all_times_out_when_task_stuck
#   5. shutdown_all_aborts_pending_handles
#
# Note: Rust identifiers cannot start with a digit, so the 100-concurrent
# test function is named `concurrent_100_inserts_all_under_10ms` rather
# than `100_concurrent_inserts_all_under_10ms`.
#
# The strict anchor `^#\[test\]` is used so `#[tokio::test]` and inline
# attributes are not counted.

set -euo pipefail

cd /home/unixdude/Archon-projects/archon-cli-worktrees/archonfixes

CRATE_DIR="crates/archon-tui-test-support"
CARGO_TOML="${CRATE_DIR}/Cargo.toml"
LIB_RS="${CRATE_DIR}/src/lib.rs"
FAKE_REGISTRY_RS="${CRATE_DIR}/src/fake_registry.rs"
TEST_FILE="${CRATE_DIR}/tests/fake_registry_smoke.rs"

# ---- 1. Cargo.toml exists -----------------------------------------------
echo "ASSERT 1: ${CARGO_TOML} exists"
if [[ ! -f "${CARGO_TOML}" ]]; then
  echo "FAIL: missing ${CARGO_TOML}" >&2
  exit 1
fi

# ---- 2. Cargo.toml has dashmap = "5" under [dependencies] ---------------
echo "ASSERT 2: ${CARGO_TOML} has dashmap = \"5\" under [dependencies]"
if ! awk '
  /^\[dependencies\]/ { in_dep = 1; next }
  /^\[/ { in_dep = 0 }
  in_dep && /^[[:space:]]*dashmap[[:space:]]*=/ { print }
' "${CARGO_TOML}" | grep -q '"5"'; then
  echo "FAIL: ${CARGO_TOML} missing dashmap = \"5\" under [dependencies]" >&2
  exit 1
fi

# ---- 3. fake_registry.rs exists -----------------------------------------
echo "ASSERT 3: ${FAKE_REGISTRY_RS} exists"
if [[ ! -f "${FAKE_REGISTRY_RS}" ]]; then
  echo "FAIL: missing ${FAKE_REGISTRY_RS}" >&2
  exit 1
fi

# ---- 4. fake_registry.rs declares pub struct FakeBackgroundAgent --------
echo "ASSERT 4: ${FAKE_REGISTRY_RS} declares pub struct FakeBackgroundAgent"
if ! grep -qE 'pub struct FakeBackgroundAgent\b' "${FAKE_REGISTRY_RS}"; then
  echo "FAIL: ${FAKE_REGISTRY_RS} missing pub struct FakeBackgroundAgent" >&2
  exit 1
fi

# ---- 5. FakeBackgroundAgent has pub id / started_at / handle fields -----
echo "ASSERT 5: FakeBackgroundAgent has pub id:, pub started_at:, pub handle:"
for field in 'id' 'started_at' 'handle'; do
  if ! grep -qE "pub ${field}:" "${FAKE_REGISTRY_RS}"; then
    echo "FAIL: ${FAKE_REGISTRY_RS} missing FakeBackgroundAgent field 'pub ${field}:'" >&2
    exit 1
  fi
done

# ---- 6. FakeBackgroundAgent has pub done_rx: Mutex<Option<...>> ---------
echo "ASSERT 6: ${FAKE_REGISTRY_RS} has pub done_rx: Mutex<Option<...>>"
if ! grep -qE 'pub done_rx:[[:space:]]*Mutex<Option<' "${FAKE_REGISTRY_RS}"; then
  echo "FAIL: ${FAKE_REGISTRY_RS} missing 'pub done_rx: Mutex<Option<...>>'" >&2
  exit 1
fi

# ---- 7. fake_registry.rs declares pub struct FakeRegistry ---------------
echo "ASSERT 7: ${FAKE_REGISTRY_RS} declares pub struct FakeRegistry"
if ! grep -qE 'pub struct FakeRegistry\b' "${FAKE_REGISTRY_RS}"; then
  echo "FAIL: ${FAKE_REGISTRY_RS} missing pub struct FakeRegistry" >&2
  exit 1
fi

# ---- 8. fake_registry.rs declares pub enum PollStatus -------------------
echo "ASSERT 8: ${FAKE_REGISTRY_RS} declares pub enum PollStatus"
if ! grep -qE 'pub enum PollStatus\b' "${FAKE_REGISTRY_RS}"; then
  echo "FAIL: ${FAKE_REGISTRY_RS} missing pub enum PollStatus" >&2
  exit 1
fi

# ---- 9. PollStatus has variants Running, Done(, NotFound ----------------
echo "ASSERT 9: PollStatus has variants Running, Done(, NotFound"
if ! grep -qE '\bRunning\b' "${FAKE_REGISTRY_RS}"; then
  echo "FAIL: ${FAKE_REGISTRY_RS} missing PollStatus variant Running" >&2
  exit 1
fi
if ! grep -qE '\bDone\(' "${FAKE_REGISTRY_RS}"; then
  echo "FAIL: ${FAKE_REGISTRY_RS} missing PollStatus variant Done(" >&2
  exit 1
fi
if ! grep -qE '\bNotFound\b' "${FAKE_REGISTRY_RS}"; then
  echo "FAIL: ${FAKE_REGISTRY_RS} missing PollStatus variant NotFound" >&2
  exit 1
fi

# ---- 10. impl FakeRegistry block exists ---------------------------------
echo "ASSERT 10: ${FAKE_REGISTRY_RS} has impl FakeRegistry"
if ! grep -qE 'impl FakeRegistry\b' "${FAKE_REGISTRY_RS}"; then
  echo "FAIL: ${FAKE_REGISTRY_RS} missing impl FakeRegistry block" >&2
  exit 1
fi

# ---- 11. FakeRegistry sync methods: new, insert, poll, len, shutdown_all
echo "ASSERT 11: FakeRegistry has pub fn new / insert / poll / len / shutdown_all"
for method in new insert poll len shutdown_all; do
  if ! grep -qE "pub fn ${method}\b" "${FAKE_REGISTRY_RS}"; then
    echo "FAIL: ${FAKE_REGISTRY_RS} missing pub fn ${method} on FakeRegistry" >&2
    exit 1
  fi
done

# ---- 12. FakeRegistry has pub async fn await_all -----------------------
echo "ASSERT 12: ${FAKE_REGISTRY_RS} has pub async fn await_all"
if ! grep -qE 'pub async fn await_all\b' "${FAKE_REGISTRY_RS}"; then
  echo "FAIL: ${FAKE_REGISTRY_RS} missing pub async fn await_all on FakeRegistry" >&2
  exit 1
fi

# ---- 13. fake_registry.rs declares pub struct AwaitAllReport -----------
echo "ASSERT 13: ${FAKE_REGISTRY_RS} declares pub struct AwaitAllReport"
if ! grep -qE 'pub struct AwaitAllReport\b' "${FAKE_REGISTRY_RS}"; then
  echo "FAIL: ${FAKE_REGISTRY_RS} missing pub struct AwaitAllReport" >&2
  exit 1
fi

# ---- 14. AwaitAllReport has pub completed:, pending:, timed_out: --------
echo "ASSERT 14: AwaitAllReport has pub completed:, pub pending:, pub timed_out:"
for field in 'completed' 'pending' 'timed_out'; do
  if ! grep -qE "pub ${field}:" "${FAKE_REGISTRY_RS}"; then
    echo "FAIL: ${FAKE_REGISTRY_RS} missing AwaitAllReport field 'pub ${field}:'" >&2
    exit 1
  fi
done

# ---- 15. pub fn spawn_fake_subagent ------------------------------------
echo "ASSERT 15: ${FAKE_REGISTRY_RS} declares pub fn spawn_fake_subagent"
if ! grep -qE 'pub fn spawn_fake_subagent\b' "${FAKE_REGISTRY_RS}"; then
  echo "FAIL: ${FAKE_REGISTRY_RS} missing pub fn spawn_fake_subagent" >&2
  exit 1
fi

# ---- 16. pub fn spawn_n_fake_subagents ---------------------------------
echo "ASSERT 16: ${FAKE_REGISTRY_RS} declares pub fn spawn_n_fake_subagents"
if ! grep -qE 'pub fn spawn_n_fake_subagents\b' "${FAKE_REGISTRY_RS}"; then
  echo "FAIL: ${FAKE_REGISTRY_RS} missing pub fn spawn_n_fake_subagents" >&2
  exit 1
fi

# ---- 17. lib.rs still declares pub mod fake_registry; -------------------
echo "ASSERT 17: ${LIB_RS} still declares pub mod fake_registry;"
if [[ ! -f "${LIB_RS}" ]]; then
  echo "FAIL: missing ${LIB_RS}" >&2
  exit 1
fi
if ! grep -qE '^\s*pub mod fake_registry\s*;' "${LIB_RS}"; then
  echo "FAIL: ${LIB_RS} missing 'pub mod fake_registry;' declaration" >&2
  exit 1
fi

# ---- 18. tests/fake_registry_smoke.rs exists ----------------------------
echo "ASSERT 18: ${TEST_FILE} exists"
if [[ ! -f "${TEST_FILE}" ]]; then
  echo "FAIL: missing ${TEST_FILE}" >&2
  exit 1
fi

# ---- 19. fake_registry_smoke.rs has exactly 5 #[test] attributes --------
echo "ASSERT 19: ${TEST_FILE} has exactly 5 #[test] attributes (strict anchor)"
TEST_COUNT=$(grep -c '^#\[test\]' "${TEST_FILE}" || true)
if [[ "${TEST_COUNT}" != "5" ]]; then
  echo "FAIL: ${TEST_FILE} expected 5 #[test] attributes, found ${TEST_COUNT}" >&2
  exit 1
fi

# ---- 20. fake_registry_smoke.rs contains expected test fn names ---------
echo "ASSERT 20: ${TEST_FILE} contains all 5 expected test fn names"
for fn in \
  'insert_then_poll_returns_done_after_task_completes' \
  'poll_unknown_id_returns_notfound' \
  'concurrent_100_inserts_all_under_10ms' \
  'await_all_times_out_when_task_stuck' \
  'shutdown_all_aborts_pending_handles'; do
  if ! grep -qE "fn ${fn}\b" "${TEST_FILE}"; then
    echo "FAIL: ${TEST_FILE} missing fn ${fn}" >&2
    exit 1
  fi
done

# ---- 21. no scope creep into src/main.rs, crates/archon-tui/src/, or
#         crates/archon-tools/ -------------------------------------------
echo "ASSERT 21: src/main.rs, crates/archon-tui/src/, and crates/archon-tools/ untouched"
SCOPE_CREEP=$(git diff --name-only src/main.rs crates/archon-tui/src/ crates/archon-tools/ 2>/dev/null | wc -l | tr -d ' ')
if [[ "${SCOPE_CREEP}" != "0" ]]; then
  echo "FAIL: src/main.rs, crates/archon-tui/src/, or crates/archon-tools/ was modified (out of scope for TUI-010)" >&2
  git diff --name-only src/main.rs crates/archon-tui/src/ crates/archon-tools/ 2>/dev/null >&2 || true
  exit 1
fi

echo "OK: verify-TUI-010 passed"
