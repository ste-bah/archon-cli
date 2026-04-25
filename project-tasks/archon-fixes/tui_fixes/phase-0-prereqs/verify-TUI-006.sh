#!/usr/bin/env bash
# Reference: project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/TASK-TUI-006.md
set -euo pipefail

cd /home/unixdude/Archon-projects/archon-cli-worktrees/archonfixes

CRATE_DIR="crates/archon-tui-test-support"
CARGO_TOML="${CRATE_DIR}/Cargo.toml"
INSTA_WRAPPER="${CRATE_DIR}/src/insta_wrapper.rs"
FIXTURES="${CRATE_DIR}/src/fixtures.rs"
TEST_FILE="${CRATE_DIR}/tests/snapshot_baselines.rs"

# --- Gate 1.1: test file exists -------------------------------------------
if [[ ! -f "${TEST_FILE}" ]]; then
  echo "FAIL: missing ${TEST_FILE}" >&2
  exit 1
fi

# --- Gate 1.2: Cargo.toml dev-dependencies contains insta -----------------
if [[ ! -f "${CARGO_TOML}" ]]; then
  echo "FAIL: missing ${CARGO_TOML}" >&2
  exit 1
fi
if ! grep -E '^\[dev-dependencies\]' "${CARGO_TOML}" >/dev/null; then
  echo "FAIL: ${CARGO_TOML} missing [dev-dependencies] section" >&2
  exit 1
fi
if ! grep -E 'insta' "${CARGO_TOML}" >/dev/null; then
  echo "FAIL: ${CARGO_TOML} missing insta dependency" >&2
  exit 1
fi

# --- Gate 1.3: insta_wrapper.rs exports required functions ----------------
if [[ ! -f "${INSTA_WRAPPER}" ]]; then
  echo "FAIL: missing ${INSTA_WRAPPER}" >&2
  exit 1
fi
if ! grep -E 'pub fn assert_buffer_snapshot' "${INSTA_WRAPPER}" >/dev/null; then
  echo "FAIL: ${INSTA_WRAPPER} missing pub fn assert_buffer_snapshot" >&2
  exit 1
fi
if ! grep -E 'pub fn redact_dynamic' "${INSTA_WRAPPER}" >/dev/null; then
  echo "FAIL: ${INSTA_WRAPPER} missing pub fn redact_dynamic" >&2
  exit 1
fi

# --- Gate 1.4: fixtures.rs exports all 5 fixture functions ----------------
if [[ ! -f "${FIXTURES}" ]]; then
  echo "FAIL: missing ${FIXTURES}" >&2
  exit 1
fi
for fn in splash_screen_buffer idle_prompt_buffer inflight_agent_buffer error_toast_buffer modal_overlay_buffer; do
  if ! grep -E "pub fn ${fn}" "${FIXTURES}" >/dev/null; then
    echo "FAIL: ${FIXTURES} missing pub fn ${fn}" >&2
    exit 1
  fi
done

# --- Gate 1.5: exactly 5 #[test] attributes in snapshot_baselines.rs ------
TEST_COUNT=$(grep -c '#\[test\]' "${TEST_FILE}" || true)
if [[ "${TEST_COUNT}" != "5" ]]; then
  echo "FAIL: ${TEST_FILE} expected 5 #[test] attributes, found ${TEST_COUNT}" >&2
  exit 1
fi

# --- Gate 1.6: 5 .snap baseline files exist (either location) -------------
SNAP_COUNT=$(find "${CRATE_DIR}/" -type f -name '*.snap' | wc -l | tr -d ' ')
if [[ "${SNAP_COUNT}" != "5" ]]; then
  echo "FAIL: expected 5 .snap files under ${CRATE_DIR}/, found ${SNAP_COUNT}" >&2
  exit 1
fi
for name in splash_screen idle_prompt inflight_agent error_toast modal_overlay; do
  FOUND=$(find "${CRATE_DIR}/" -type f -name "*${name}*.snap" | head -1)
  if [[ -z "${FOUND}" ]]; then
    echo "FAIL: missing baseline snapshot for '${name}' under ${CRATE_DIR}/" >&2
    exit 1
  fi
done

# --- Gate 1.7: snapshot_baselines test binary runs green ------------------
set +e
TEST_OUT=$(cargo test -p archon-tui-test-support --test snapshot_baselines -j1 -- --test-threads=2 2>&1 | tail -30)
TEST_EXIT=$?
set -e
echo "${TEST_OUT}"
if [[ "${TEST_EXIT}" != "0" ]]; then
  echo "FAIL: cargo test snapshot_baselines exited ${TEST_EXIT}" >&2
  exit 1
fi
if ! echo "${TEST_OUT}" | grep -E 'test result: ok\. [5-9][0-9]* passed|test result: ok\. 5 passed' >/dev/null; then
  if ! echo "${TEST_OUT}" | grep -E 'test result: ok\. ([5-9]|[1-9][0-9]+) passed' >/dev/null; then
    echo "FAIL: snapshot_baselines did not report >=5 passed" >&2
    exit 1
  fi
fi

# --- Gate 1.8: full crate test suite passes with INSTA_UPDATE=no ----------
set +e
FULL_OUT=$(INSTA_UPDATE=no cargo test -p archon-tui-test-support -j1 -- --test-threads=2 2>&1 | tail -15)
FULL_EXIT=$?
set -e
echo "${FULL_OUT}"
if [[ "${FULL_EXIT}" != "0" ]]; then
  echo "FAIL: INSTA_UPDATE=no cargo test -p archon-tui-test-support exited ${FULL_EXIT}" >&2
  exit 1
fi

# --- Gate 1.9: workspace still type-checks --------------------------------
set +e
CHECK_OUT=$(cargo check --workspace -j1 2>&1 | tail -10)
CHECK_EXIT=$?
set -e
echo "${CHECK_OUT}"
if [[ "${CHECK_EXIT}" != "0" ]]; then
  echo "FAIL: cargo check --workspace exited ${CHECK_EXIT}" >&2
  exit 1
fi

# --- Gate 1.10: no scope creep into src/main.rs or crates/archon-tui/src/ -
if [[ -n "$(git diff --name-only src/main.rs 2>/dev/null || true)" ]]; then
  echo "FAIL: src/main.rs was modified (out of scope for TUI-006)" >&2
  exit 1
fi
if [[ -n "$(git diff --name-only crates/archon-tui/src/ 2>/dev/null || true)" ]]; then
  echo "FAIL: crates/archon-tui/src/ was modified (out of scope for TUI-006)" >&2
  exit 1
fi

echo "OK: verify-TUI-006 passed"
