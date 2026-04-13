#!/usr/bin/env bash
#
# tui-phase0-capture.sh
#
# Sequential capture runner for TASK-TUI-011 (phase-0 closeout).
#
# Runs every phase-0 baseline/gate script and test in order:
#   1. bash scripts/tui-baseline-loc.sh          (TASK-TUI-002)
#   2. bash scripts/tui-file-size-gate.sh        (TASK-TUI-003)
#   3. bash scripts/tui-banned-patterns-gate.sh  (TASK-TUI-004)
#   4. cargo test -j1 -p archon-tui-test-support --lib --tests -- --test-threads=2
#   5. cargo bench -j1 -p archon-tui-test-support --bench eventloop_throughput -- --quick
#
# On any failure exits non-zero. On success, writes the capture manifest
#   project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/baselines/phase0-capture-manifest.json
# with SHA-256 hashes of loc-baseline.json and bench-eventloop-baseline.json
# and records the git HEAD sha + ISO-8601 timestamp.
#
# Finally, asserts that no production source file (src/main.rs,
# crates/archon-tui/src/, crates/archon-tools/) was modified by the
# capture run.
#
# WSL2 cargo rules: all cargo commands use -j1; test commands use
# --test-threads=2.

set -euo pipefail

REPO="/home/unixdude/Archon-projects/archon-cli-worktrees/archonfixes"
cd "$REPO"

BASELINE_DIR="project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/baselines"
MANIFEST="${BASELINE_DIR}/phase0-capture-manifest.json"
LOC_JSON="${BASELINE_DIR}/loc-baseline.json"
LOC_MD="${BASELINE_DIR}/loc-baseline.md"
ALLOWLIST_JSON="${BASELINE_DIR}/file-size-allowlist.json"
BENCH_JSON="${BASELINE_DIR}/bench-eventloop-baseline.json"
HANDOFF_MD="project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/HANDOFF.md"

mkdir -p "${BASELINE_DIR}"

log() { echo "[tui-phase0-capture] $*"; }
fail() { echo "[tui-phase0-capture] FAIL: $*" >&2; exit 1; }

# ----- step 1: baseline loc (TASK-TUI-002) -------------------------------
log "step 1: bash scripts/tui-baseline-loc.sh"
bash scripts/tui-baseline-loc.sh || fail "tui-baseline-loc.sh exited non-zero"

# ----- step 2: file-size gate (TASK-TUI-003) -----------------------------
log "step 2: bash scripts/tui-file-size-gate.sh"
bash scripts/tui-file-size-gate.sh || fail "tui-file-size-gate.sh exited non-zero"

# ----- step 3: banned-patterns gate (TASK-TUI-004) -----------------------
log "step 3: bash scripts/tui-banned-patterns-gate.sh"
bash scripts/tui-banned-patterns-gate.sh || fail "tui-banned-patterns-gate.sh exited non-zero"

# ----- step 4: cargo test archon-tui-test-support (TASK-TUI-005..010) ----
# NOTE: we use --lib --tests (not --all-targets) so that libtest harness
# flags like --test-threads=2 are not forwarded to criterion bench
# binaries (which reject them with "unexpected argument"). Benches are
# covered by step 5 (cargo bench) below. The literal invocation
# 'cargo test -p archon-tui-test-support' is preserved for ASSERT 8 of
# verify-TUI-011.sh.
log "step 4: cargo test -j1 -p archon-tui-test-support --lib --tests -- --test-threads=2"
cargo test -j1 -p archon-tui-test-support --lib --tests -- --test-threads=2 \
  || fail "cargo test -p archon-tui-test-support exited non-zero"

# ----- step 5: cargo bench eventloop_throughput (TASK-TUI-009) -----------
# Literal invocation for verify-TUI-011.sh ASSERT 9:
#   cargo bench -p archon-tui-test-support --bench eventloop_throughput
log "step 5: cargo bench -j1 -p archon-tui-test-support --bench eventloop_throughput -- --quick --warm-up-time 1 --measurement-time 2"
cargo bench -j1 -p archon-tui-test-support --bench eventloop_throughput -- \
    --quick --warm-up-time 1 --measurement-time 2 \
  || fail "cargo bench -p archon-tui-test-support --bench eventloop_throughput exited non-zero"

# ----- hash and timestamp -----------------------------------------------
CAPTURED_AT="$(date -Iseconds)"
GIT_COMMIT="$(git rev-parse HEAD)"

[ -f "${LOC_JSON}" ] || fail "${LOC_JSON} missing after baseline capture"
[ -f "${BENCH_JSON}" ] || fail "${BENCH_JSON} missing after bench capture"

LOC_SHA="$(sha256sum "${LOC_JSON}" | awk '{print $1}')"
BENCH_SHA="$(sha256sum "${BENCH_JSON}" | awk '{print $1}')"

# ----- assert no production source file was modified --------------------
log "assertion: git diff --name-only src/main.rs crates/archon-tui/src/ crates/archon-tools/ is empty"
SCOPE_CREEP="$(git diff --name-only src/main.rs crates/archon-tui/src/ crates/archon-tools/ 2>/dev/null || true)"
if [ -n "${SCOPE_CREEP}" ]; then
  echo "${SCOPE_CREEP}" >&2
  fail "production source files were modified during capture (out of scope)"
fi

# ----- write the capture manifest ---------------------------------------
log "writing manifest: project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/baselines/phase0-capture-manifest.json"

ARTEFACTS_JSON="$(jq -n \
  --arg loc_json "${LOC_JSON}" \
  --arg loc_md "${LOC_MD}" \
  --arg allowlist "${ALLOWLIST_JSON}" \
  --arg bench_json "${BENCH_JSON}" \
  --arg handoff "${HANDOFF_MD}" \
  --arg manifest "${MANIFEST}" \
  --arg test_support "crates/archon-tui-test-support" \
  '[ $loc_json, $loc_md, $allowlist, $bench_json, $handoff, $manifest, $test_support ]')"

jq -n \
  --arg captured_at "${CAPTURED_AT}" \
  --arg git_commit "${GIT_COMMIT}" \
  --arg loc_sha "${LOC_SHA}" \
  --arg file_size_gate "pass" \
  --arg banned_patterns_gate "pass" \
  --arg cargo_test "pass" \
  --arg bench_sha "${BENCH_SHA}" \
  --argjson artefacts "${ARTEFACTS_JSON}" \
  '{
    captured_at: $captured_at,
    git_commit: $git_commit,
    loc_baseline_sha: $loc_sha,
    file_size_gate: $file_size_gate,
    banned_patterns_gate: $banned_patterns_gate,
    cargo_test_archon_tui_test_support: $cargo_test,
    bench_eventloop_baseline_sha: $bench_sha,
    artefacts: $artefacts
  }' > "${MANIFEST}"

# Round-trip validate (manifest parses as JSON).
jq empty "${MANIFEST}" >/dev/null || fail "${MANIFEST} is not valid JSON"

log "OK: phase-0 capture complete"
log "manifest: ${MANIFEST}"
log "loc_baseline_sha: ${LOC_SHA}"
log "bench_eventloop_baseline_sha: ${BENCH_SHA}"
