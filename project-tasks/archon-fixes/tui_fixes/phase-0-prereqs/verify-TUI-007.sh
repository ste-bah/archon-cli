#!/usr/bin/env bash
# Reference: project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/TASK-TUI-007.md
set -euo pipefail

cd /home/unixdude/Archon-projects/archon-cli-worktrees/archonfixes

CRATE_DIR="crates/archon-tui-test-support"
CARGO_TOML="${CRATE_DIR}/Cargo.toml"
TEST_BACKEND="${CRATE_DIR}/src/test_backend.rs"
TEST_FILE="${CRATE_DIR}/tests/test_backend_smoke.rs"

# ---- 1. test_backend.rs exists ------------------------------------------
if [[ ! -f "${TEST_BACKEND}" ]]; then
  echo "FAIL: missing ${TEST_BACKEND}" >&2
  exit 1
fi

# ---- 2. contains pub struct TuiHarness ----------------------------------
if ! grep -qE 'pub struct TuiHarness\b' "${TEST_BACKEND}"; then
  echo "FAIL: ${TEST_BACKEND} missing pub struct TuiHarness" >&2
  exit 1
fi

# ---- 3. contains pub struct TuiHarnessBuilder ---------------------------
if ! grep -qE 'pub struct TuiHarnessBuilder\b' "${TEST_BACKEND}"; then
  echo "FAIL: ${TEST_BACKEND} missing pub struct TuiHarnessBuilder" >&2
  exit 1
fi

# ---- 4. contains pub struct FakeInputStream -----------------------------
if ! grep -qE 'pub struct FakeInputStream\b' "${TEST_BACKEND}"; then
  echo "FAIL: ${TEST_BACKEND} missing pub struct FakeInputStream" >&2
  exit 1
fi

# ---- 5. contains pub fn builder -----------------------------------------
if ! grep -qE 'pub fn builder\s*\(' "${TEST_BACKEND}"; then
  echo "FAIL: ${TEST_BACKEND} missing pub fn builder(...)" >&2
  exit 1
fi

# ---- 6. contains pub fn render_frame ------------------------------------
if ! grep -qE 'pub fn render_frame\b' "${TEST_BACKEND}"; then
  echo "FAIL: ${TEST_BACKEND} missing pub fn render_frame" >&2
  exit 1
fi

# ---- 7. contains pub fn buffer(&self) -----------------------------------
if ! grep -qE 'pub fn buffer\s*\(\s*&self' "${TEST_BACKEND}"; then
  echo "FAIL: ${TEST_BACKEND} missing pub fn buffer(&self, ...)" >&2
  exit 1
fi

# ---- 8. contains pub fn resize ------------------------------------------
if ! grep -qE 'pub fn resize\b' "${TEST_BACKEND}"; then
  echo "FAIL: ${TEST_BACKEND} missing pub fn resize" >&2
  exit 1
fi

# ---- 9. contains pub fn measure_render_latency --------------------------
if ! grep -qE 'pub fn measure_render_latency\b' "${TEST_BACKEND}"; then
  echo "FAIL: ${TEST_BACKEND} missing pub fn measure_render_latency" >&2
  exit 1
fi

# ---- 10. contains fn size (builder method) ------------------------------
if ! grep -qE '\bfn size\s*\(' "${TEST_BACKEND}"; then
  echo "FAIL: ${TEST_BACKEND} missing fn size(...) on builder" >&2
  exit 1
fi

# ---- 11. contains fn paused_clock (builder method) ----------------------
if ! grep -qE '\bfn paused_clock\s*\(' "${TEST_BACKEND}"; then
  echo "FAIL: ${TEST_BACKEND} missing fn paused_clock(...) on builder" >&2
  exit 1
fi

# ---- 12. contains fn build (builder method) -----------------------------
if ! grep -qE '\bfn build\s*\(' "${TEST_BACKEND}"; then
  echo "FAIL: ${TEST_BACKEND} missing fn build(...) on builder" >&2
  exit 1
fi

# ---- 13. test_backend_smoke.rs exists -----------------------------------
if [[ ! -f "${TEST_FILE}" ]]; then
  echo "FAIL: missing ${TEST_FILE}" >&2
  exit 1
fi

# ---- 14. exactly 3 #[test] attributes -----------------------------------
TEST_COUNT=$(grep -c '^#\[test\]' "${TEST_FILE}" || true)
if [[ "${TEST_COUNT}" != "3" ]]; then
  echo "FAIL: ${TEST_FILE} expected 3 #[test] attributes, found ${TEST_COUNT}" >&2
  exit 1
fi

# ---- 15. contains fn render_frame_produces_non_empty_buffer -------------
if ! grep -qE 'fn render_frame_produces_non_empty_buffer\b' "${TEST_FILE}"; then
  echo "FAIL: ${TEST_FILE} missing fn render_frame_produces_non_empty_buffer" >&2
  exit 1
fi

# ---- 16. contains fn resize_changes_dimensions --------------------------
if ! grep -qE 'fn resize_changes_dimensions\b' "${TEST_FILE}"; then
  echo "FAIL: ${TEST_FILE} missing fn resize_changes_dimensions" >&2
  exit 1
fi

# ---- 17. contains fn measure_render_latency_under_paused_clock_is_deterministic
if ! grep -qE 'fn measure_render_latency_under_paused_clock_is_deterministic\b' "${TEST_FILE}"; then
  echo "FAIL: ${TEST_FILE} missing fn measure_render_latency_under_paused_clock_is_deterministic" >&2
  exit 1
fi

# ---- 18. Cargo.toml has ratatui under [dependencies] --------------------
if [[ ! -f "${CARGO_TOML}" ]]; then
  echo "FAIL: missing ${CARGO_TOML}" >&2
  exit 1
fi
if ! awk '
  /^\[dependencies\]/ { in_deps = 1; next }
  /^\[/ { in_deps = 0 }
  in_deps && /^[[:space:]]*ratatui[[:space:]]*=/ { found = 1 }
  END { exit(found ? 0 : 1) }
' "${CARGO_TOML}"; then
  echo "FAIL: ${CARGO_TOML} missing ratatui under [dependencies]" >&2
  exit 1
fi

# ---- 19. Cargo.toml has tokio with test-util feature --------------------
if ! grep -qE 'tokio.*test-util' "${CARGO_TOML}"; then
  echo "FAIL: ${CARGO_TOML} missing tokio with test-util feature" >&2
  exit 1
fi

# ---- 20. cargo test test_backend_smoke passes ---------------------------
set +e
TEST_OUT=$(cargo test -p archon-tui-test-support --test test_backend_smoke -j1 -- --test-threads=2 2>&1 | tail -30)
TEST_EXIT=$?
set -e
echo "${TEST_OUT}"
if [[ "${TEST_EXIT}" != "0" ]]; then
  echo "FAIL: cargo test test_backend_smoke exited ${TEST_EXIT}" >&2
  exit 1
fi

# ---- 21. workspace still type-checks ------------------------------------
set +e
CHECK_OUT=$(cargo check --workspace -j1 2>&1 | tail -10)
CHECK_EXIT=$?
set -e
echo "${CHECK_OUT}"
if [[ "${CHECK_EXIT}" != "0" ]]; then
  echo "FAIL: cargo check --workspace exited ${CHECK_EXIT}" >&2
  exit 1
fi

# ---- 22. no scope creep into src/main.rs or crates/archon-tui/src/ ------
SCOPE_CREEP=$(git diff --name-only src/main.rs crates/archon-tui/src/ 2>/dev/null | wc -l | tr -d ' ')
if [[ "${SCOPE_CREEP}" != "0" ]]; then
  echo "FAIL: src/main.rs or crates/archon-tui/src/ was modified (out of scope for TUI-007)" >&2
  git diff --name-only src/main.rs crates/archon-tui/src/ 2>/dev/null >&2 || true
  exit 1
fi

echo "OK: verify-TUI-007 passed"
