#!/usr/bin/env bash
# Reference: project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/TASK-TUI-005.md
set -euo pipefail

cd /home/unixdude/Archon-projects/archon-cli-worktrees/archonfixes

CRATE_DIR="crates/archon-tui-test-support"
SRC_DIR="${CRATE_DIR}/src"

# --- 1. Assert all 8 crate files exist ---
REQUIRED_FILES=(
  "${CRATE_DIR}/Cargo.toml"
  "${SRC_DIR}/lib.rs"
  "${SRC_DIR}/mock_agent.rs"
  "${SRC_DIR}/test_backend.rs"
  "${SRC_DIR}/fake_registry.rs"
  "${SRC_DIR}/metrics.rs"
  "${SRC_DIR}/fixtures.rs"
  "${SRC_DIR}/insta_wrapper.rs"
)

for f in "${REQUIRED_FILES[@]}"; do
  if [[ ! -f "${f}" ]]; then
    echo "FAIL: required file missing: ${f}" >&2
    exit 1
  fi
done

# --- 2. Assert Cargo.toml contains name and publish = false ---
CARGO_TOML="${CRATE_DIR}/Cargo.toml"
if ! grep -qE '^name[[:space:]]*=[[:space:]]*"archon-tui-test-support"' "${CARGO_TOML}"; then
  echo "FAIL: Cargo.toml missing name = \"archon-tui-test-support\"" >&2
  exit 1
fi
if ! grep -qE '^publish[[:space:]]*=[[:space:]]*false' "${CARGO_TOML}"; then
  echo "FAIL: Cargo.toml missing publish = false" >&2
  exit 1
fi

# --- 3. Assert lib.rs declares exactly 6 public modules ---
LIB_RS="${SRC_DIR}/lib.rs"
PUB_MOD_COUNT=$(grep -cE '^pub mod ' "${LIB_RS}" || true)
if [[ "${PUB_MOD_COUNT}" -ne 6 ]]; then
  echo "FAIL: lib.rs must declare exactly 6 public modules (found ${PUB_MOD_COUNT})" >&2
  exit 1
fi

EXPECTED_MODS=(mock_agent test_backend fake_registry metrics fixtures insta_wrapper)
for mod in "${EXPECTED_MODS[@]}"; do
  if ! grep -qE "^pub mod ${mod}( |;)" "${LIB_RS}"; then
    echo "FAIL: lib.rs missing 'pub mod ${mod}'" >&2
    exit 1
  fi
done

# --- 4. Assert each of the 6 placeholder .rs files has a leading comment (//! or //) ---
PLACEHOLDER_FILES=(
  "${SRC_DIR}/mock_agent.rs"
  "${SRC_DIR}/test_backend.rs"
  "${SRC_DIR}/fake_registry.rs"
  "${SRC_DIR}/metrics.rs"
  "${SRC_DIR}/fixtures.rs"
  "${SRC_DIR}/insta_wrapper.rs"
)
for f in "${PLACEHOLDER_FILES[@]}"; do
  if ! grep -qE '^//' "${f}"; then
    echo "FAIL: ${f} must contain a leading // or //! comment line" >&2
    exit 1
  fi
done

# --- 5. Assert src/main.rs is not modified ---
if [[ -n "$(git diff --name-only src/main.rs 2>/dev/null || true)" ]]; then
  echo "FAIL: src/main.rs must not be modified by TUI-005" >&2
  exit 1
fi

# --- 6. Assert crates/archon-tui/src/ is not modified ---
if [[ -n "$(git diff --name-only crates/archon-tui/src/ 2>/dev/null || true)" ]]; then
  echo "FAIL: crates/archon-tui/src/ must not be modified by TUI-005" >&2
  exit 1
fi

# --- 7. cargo check on the new crate (WSL2: always -j1) ---
echo "Running: cargo check -p archon-tui-test-support -j1"
set +e
CHECK_OUT=$(cargo check -p archon-tui-test-support -j1 2>&1)
CHECK_RC=$?
set -e
echo "${CHECK_OUT}" | tail -20
if [[ "${CHECK_RC}" -ne 0 ]]; then
  echo "FAIL: cargo check -p archon-tui-test-support exited ${CHECK_RC}" >&2
  exit 1
fi

# --- 8. cargo check --workspace (WSL2: always -j1) ---
echo "Running: cargo check --workspace -j1"
set +e
WS_OUT=$(cargo check --workspace -j1 2>&1)
WS_RC=$?
set -e
echo "${WS_OUT}" | tail -20
if [[ "${WS_RC}" -ne 0 ]]; then
  echo "FAIL: cargo check --workspace exited ${WS_RC}" >&2
  exit 1
fi

echo "OK: verify-TUI-005 passed"
