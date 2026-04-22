#!/usr/bin/env bash
# Self-test for check-complexity.sh
# Case A: fixture function with cognitive complexity >= 12 must cause
#         clippy (with -D clippy::cognitive_complexity and threshold=10)
#         to exit non-zero and mention `cognitive_complexity` in output.
# Case B: trivial function must pass (exit 0).
#
# We invoke `cargo clippy` directly inside a copy of the complex/ fixture
# instead of running `check-complexity.sh` itself, because the real script
# targets `-p archon-tui` in the parent workspace. The flag surface under
# test (-D clippy::cognitive_complexity + clippy.toml threshold) is
# identical, and this is the only way to get a hermetic, focused run.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECKER="${SCRIPT_DIR}/../check-complexity.sh"
FIXTURE_SRC="${SCRIPT_DIR}/fixtures/complex"

if [[ ! -x "$CHECKER" ]]; then
    echo "FAIL: check-complexity.sh not executable at $CHECKER"
    exit 1
fi

if [[ ! -d "$FIXTURE_SRC" ]]; then
    echo "FAIL: fixture dir missing at $FIXTURE_SRC"
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "FAIL: cargo not available on PATH — cannot exercise clippy"
    exit 1
fi

WORK="$(mktemp -d -t complexity-self-test-XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

cp -R "$FIXTURE_SRC" "$WORK/complex"
# Make sure the fixture carries a clippy.toml mirroring the one at workspace
# root, so the threshold is applied hermetically even if the copy is
# re-rooted out of the archon worktree.
cat > "$WORK/complex/clippy.toml" <<'TOML'
cognitive-complexity-threshold = 10
TOML

run_clippy() {
    (
        cd "$WORK/complex"
        cargo clippy --offline -j1 --all-targets \
            -- -D clippy::cognitive_complexity 2>&1 \
            || cargo clippy -j1 --all-targets \
                -- -D clippy::cognitive_complexity 2>&1
    )
}

# --- Case A: complex fixture must fail ---
echo "--- Case A: complex fixture (expect rc != 0 + cognitive_complexity) ---"
set +e
OUT_A="$(run_clippy)"
RC_A=$?
set -e
echo "$OUT_A" | tail -n 40
if [ "$RC_A" -eq 0 ]; then
    echo "TEST FAIL: complex fixture accepted (should have failed)"
    exit 1
fi
if ! printf '%s' "$OUT_A" | grep -q 'cognitive_complexity'; then
    echo "TEST FAIL: output did not mention cognitive_complexity"
    exit 1
fi
echo "PASS complex-fails (rc=$RC_A)"

# --- Case B: strip fixture down to a trivial function, must pass ---
echo "--- Case B: trivial fixture (expect rc == 0) ---"
cat > "$WORK/complex/src/lib.rs" <<'RS'
pub fn noop(x: i32) -> i32 { x + 1 }
RS

set +e
OUT_B="$(run_clippy)"
RC_B=$?
set -e
echo "$OUT_B" | tail -n 40
if [ "$RC_B" -ne 0 ]; then
    echo "TEST FAIL: trivial fixture rejected (should have passed)"
    exit 1
fi
echo "PASS clean-ok (rc=$RC_B)"

echo "ALL TESTS PASSED"
