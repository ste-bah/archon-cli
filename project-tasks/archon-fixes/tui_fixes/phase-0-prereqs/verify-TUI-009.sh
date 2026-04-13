#!/usr/bin/env bash
# Reference: project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/TASK-TUI-009.md
#
# Gate 1 static-assertion verifier for TASK-TUI-009.
#
# This script enforces the acceptance criteria of TASK-TUI-009 statically
# (file existence, grep, jq) without running cargo. It is intentionally
# strict: every assertion is numbered and emits a clear FAIL message.
#
# Expected test count in tests/metrics_smoke.rs is 4:
#   1. recorder_new_then_observe_drain_nonzero_snapshot
#   2. sample_backlog_tracks_peaks
#   3. assert_linear_memory_growth_pass_on_1mb_per_1000
#   4. assert_linear_memory_growth_fail_on_1mb_per_10
#
# The strict anchor `^#\[test\]` is used so `#[tokio::test]` and inline
# attributes are not counted.

set -euo pipefail

cd /home/unixdude/Archon-projects/archon-cli-worktrees/archonfixes

CRATE_DIR="crates/archon-tui-test-support"
CARGO_TOML="${CRATE_DIR}/Cargo.toml"
LIB_RS="${CRATE_DIR}/src/lib.rs"
METRICS_RS="${CRATE_DIR}/src/metrics.rs"
BENCH_FILE="${CRATE_DIR}/benches/eventloop_throughput.rs"
TEST_FILE="${CRATE_DIR}/tests/metrics_smoke.rs"
BASELINE_JSON="project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/baselines/bench-eventloop-baseline.json"

# ---- 1. Cargo.toml exists -----------------------------------------------
echo "ASSERT 1: ${CARGO_TOML} exists"
if [[ ! -f "${CARGO_TOML}" ]]; then
  echo "FAIL: missing ${CARGO_TOML}" >&2
  exit 1
fi

# ---- 2. Cargo.toml has criterion 0.5 with html_reports feature ----------
echo "ASSERT 2: ${CARGO_TOML} has criterion 0.5 with html_reports feature under [dev-dependencies]"
if ! awk '
  /^\[dev-dependencies\]/ { in_dev = 1; next }
  /^\[/ { in_dev = 0 }
  in_dev && /^[[:space:]]*criterion[[:space:]]*=/ { print }
' "${CARGO_TOML}" | grep -q '0\.5'; then
  echo "FAIL: ${CARGO_TOML} missing criterion = { version = \"0.5\", ... } under [dev-dependencies]" >&2
  exit 1
fi
if ! awk '
  /^\[dev-dependencies\]/ { in_dev = 1; next }
  /^\[/ { in_dev = 0 }
  in_dev && /^[[:space:]]*criterion[[:space:]]*=/ { print }
' "${CARGO_TOML}" | grep -q 'html_reports'; then
  echo "FAIL: ${CARGO_TOML} criterion dep missing html_reports feature" >&2
  exit 1
fi

# ---- 3. Cargo.toml has hdrhistogram 7 under [dev-dependencies] ----------
echo "ASSERT 3: ${CARGO_TOML} has hdrhistogram = \"7\" under [dev-dependencies]"
if ! awk '
  /^\[dev-dependencies\]/ { in_dev = 1; next }
  /^\[/ { in_dev = 0 }
  in_dev && /^[[:space:]]*hdrhistogram[[:space:]]*=/ { print }
' "${CARGO_TOML}" | grep -q '"7"'; then
  echo "FAIL: ${CARGO_TOML} missing hdrhistogram = \"7\" under [dev-dependencies]" >&2
  exit 1
fi

# ---- 4. Cargo.toml has [[bench]] eventloop_throughput with harness=false -
echo "ASSERT 4: ${CARGO_TOML} has [[bench]] name = \"eventloop_throughput\" harness = false"
if ! awk '
  /^\[\[bench\]\]/ { in_bench = 1; name = ""; harness = ""; next }
  /^\[/ && !/^\[\[bench\]\]/ {
    if (in_bench && name == "\"eventloop_throughput\"" && harness == "false") found = 1
    in_bench = 0
  }
  in_bench && /^[[:space:]]*name[[:space:]]*=/ {
    sub(/^[[:space:]]*name[[:space:]]*=[[:space:]]*/, "")
    name = $0
  }
  in_bench && /^[[:space:]]*harness[[:space:]]*=/ {
    sub(/^[[:space:]]*harness[[:space:]]*=[[:space:]]*/, "")
    harness = $0
  }
  END {
    if (in_bench && name == "\"eventloop_throughput\"" && harness == "false") found = 1
    exit(found ? 0 : 1)
  }
' "${CARGO_TOML}"; then
  echo "FAIL: ${CARGO_TOML} missing [[bench]] with name=\"eventloop_throughput\" and harness=false" >&2
  exit 1
fi

# ---- 5. metrics.rs exists -----------------------------------------------
echo "ASSERT 5: ${METRICS_RS} exists"
if [[ ! -f "${METRICS_RS}" ]]; then
  echo "FAIL: missing ${METRICS_RS}" >&2
  exit 1
fi

# ---- 6. metrics.rs declares pub struct MetricsRecorder ------------------
echo "ASSERT 6: ${METRICS_RS} declares pub struct MetricsRecorder"
if ! grep -qE 'pub struct MetricsRecorder\b' "${METRICS_RS}"; then
  echo "FAIL: ${METRICS_RS} missing pub struct MetricsRecorder" >&2
  exit 1
fi

# ---- 7. metrics.rs declares pub struct DrainSample ----------------------
echo "ASSERT 7: ${METRICS_RS} declares pub struct DrainSample"
if ! grep -qE 'pub struct DrainSample\b' "${METRICS_RS}"; then
  echo "FAIL: ${METRICS_RS} missing pub struct DrainSample" >&2
  exit 1
fi

# ---- 8. DrainSample has all 4 pub fields --------------------------------
echo "ASSERT 8: DrainSample has pub t:, pub batch:, pub backlog:, pub mem_bytes:"
for field in 't' 'batch' 'backlog' 'mem_bytes'; do
  if ! grep -qE "pub ${field}:" "${METRICS_RS}"; then
    echo "FAIL: ${METRICS_RS} missing DrainSample field 'pub ${field}:'" >&2
    exit 1
  fi
done

# ---- 9. metrics.rs declares pub struct MetricsSnapshot ------------------
echo "ASSERT 9: ${METRICS_RS} declares pub struct MetricsSnapshot"
if ! grep -qE 'pub struct MetricsSnapshot\b' "${METRICS_RS}"; then
  echo "FAIL: ${METRICS_RS} missing pub struct MetricsSnapshot" >&2
  exit 1
fi

# ---- 10. MetricsSnapshot has all 7 pub fields ---------------------------
echo "ASSERT 10: MetricsSnapshot has all 7 pub fields"
for field in 'p50_us' 'p95_us' 'p99_us' 'total_events' 'peak_backlog' 'peak_mem_bytes' 'throughput_eps'; do
  if ! grep -qE "pub ${field}:" "${METRICS_RS}"; then
    echo "FAIL: ${METRICS_RS} missing MetricsSnapshot field 'pub ${field}:'" >&2
    exit 1
  fi
done

# ---- 11. impl MetricsRecorder with pub fn new ---------------------------
echo "ASSERT 11: ${METRICS_RS} has impl MetricsRecorder"
if ! grep -qE 'impl MetricsRecorder\b' "${METRICS_RS}"; then
  echo "FAIL: ${METRICS_RS} missing impl MetricsRecorder block" >&2
  exit 1
fi

# ---- 12. impl methods: new, observe_drain, sample_backlog, snapshot -----
echo "ASSERT 12: MetricsRecorder has pub fn new / observe_drain / sample_backlog / snapshot"
for method in new observe_drain sample_backlog snapshot; do
  if ! grep -qE "pub fn ${method}\b" "${METRICS_RS}"; then
    echo "FAIL: ${METRICS_RS} missing pub fn ${method} on MetricsRecorder" >&2
    exit 1
  fi
done

# ---- 13. pub fn assert_linear_memory_growth -----------------------------
echo "ASSERT 13: ${METRICS_RS} declares pub fn assert_linear_memory_growth"
if ! grep -qE 'pub fn assert_linear_memory_growth\b' "${METRICS_RS}"; then
  echo "FAIL: ${METRICS_RS} missing pub fn assert_linear_memory_growth" >&2
  exit 1
fi

# ---- 14. lib.rs still declares pub mod metrics; -------------------------
echo "ASSERT 14: ${LIB_RS} still declares pub mod metrics;"
if [[ ! -f "${LIB_RS}" ]]; then
  echo "FAIL: missing ${LIB_RS}" >&2
  exit 1
fi
if ! grep -qE '^\s*pub mod metrics\s*;' "${LIB_RS}"; then
  echo "FAIL: ${LIB_RS} missing 'pub mod metrics;' declaration" >&2
  exit 1
fi

# ---- 15. benches/eventloop_throughput.rs exists -------------------------
echo "ASSERT 15: ${BENCH_FILE} exists"
if [[ ! -f "${BENCH_FILE}" ]]; then
  echo "FAIL: missing ${BENCH_FILE}" >&2
  exit 1
fi

# ---- 16. bench file references criterion_group -------------------------
echo "ASSERT 16: ${BENCH_FILE} references criterion_group"
if ! grep -qE '\bcriterion_group\b' "${BENCH_FILE}"; then
  echo "FAIL: ${BENCH_FILE} missing criterion_group" >&2
  exit 1
fi

# ---- 17. bench file references criterion_main --------------------------
echo "ASSERT 17: ${BENCH_FILE} references criterion_main"
if ! grep -qE '\bcriterion_main\b' "${BENCH_FILE}"; then
  echo "FAIL: ${BENCH_FILE} missing criterion_main" >&2
  exit 1
fi

# ---- 18. bench file references unbounded_single_producer_single_consumer
echo "ASSERT 18: ${BENCH_FILE} references unbounded_single_producer_single_consumer"
if ! grep -qE '\bunbounded_single_producer_single_consumer\b' "${BENCH_FILE}"; then
  echo "FAIL: ${BENCH_FILE} missing unbounded_single_producer_single_consumer group" >&2
  exit 1
fi

# ---- 19. bench file references unbounded_100_producers_single_consumer -
echo "ASSERT 19: ${BENCH_FILE} references unbounded_100_producers_single_consumer"
if ! grep -qE '\bunbounded_100_producers_single_consumer\b' "${BENCH_FILE}"; then
  echo "FAIL: ${BENCH_FILE} missing unbounded_100_producers_single_consumer group" >&2
  exit 1
fi

# ---- 20. bench file references p95_event_latency_under_10k_eps ---------
echo "ASSERT 20: ${BENCH_FILE} references p95_event_latency_under_10k_eps"
if ! grep -qE '\bp95_event_latency_under_10k_eps\b' "${BENCH_FILE}"; then
  echo "FAIL: ${BENCH_FILE} missing p95_event_latency_under_10k_eps group" >&2
  exit 1
fi

# ---- 21. bench file uses MockAgent or spawn_n_mock_agents from mock_agent
echo "ASSERT 21: ${BENCH_FILE} imports MockAgent or spawn_n_mock_agents from archon_tui_test_support::mock_agent"
if ! grep -qE 'archon_tui_test_support::mock_agent' "${BENCH_FILE}"; then
  echo "FAIL: ${BENCH_FILE} missing archon_tui_test_support::mock_agent usage" >&2
  exit 1
fi
if ! grep -qE '\b(MockAgent|spawn_n_mock_agents)\b' "${BENCH_FILE}"; then
  echo "FAIL: ${BENCH_FILE} missing MockAgent or spawn_n_mock_agents reference" >&2
  exit 1
fi

# ---- 22. tests/metrics_smoke.rs exists ----------------------------------
echo "ASSERT 22: ${TEST_FILE} exists"
if [[ ! -f "${TEST_FILE}" ]]; then
  echo "FAIL: missing ${TEST_FILE}" >&2
  exit 1
fi

# ---- 23. metrics_smoke.rs has exactly 4 #[test] attributes --------------
echo "ASSERT 23: ${TEST_FILE} has exactly 4 #[test] attributes (strict anchor)"
TEST_COUNT=$(grep -c '^#\[test\]' "${TEST_FILE}" || true)
if [[ "${TEST_COUNT}" != "4" ]]; then
  echo "FAIL: ${TEST_FILE} expected 4 #[test] attributes, found ${TEST_COUNT}" >&2
  exit 1
fi

# ---- 24. metrics_smoke.rs contains expected test fn names ---------------
echo "ASSERT 24: ${TEST_FILE} contains all 4 expected test fn names"
for fn in \
  'recorder_new_then_observe_drain_nonzero_snapshot' \
  'sample_backlog_tracks_peaks' \
  'assert_linear_memory_growth_pass_on_1mb_per_1000' \
  'assert_linear_memory_growth_fail_on_1mb_per_10'; do
  if ! grep -qE "fn ${fn}\b" "${TEST_FILE}"; then
    echo "FAIL: ${TEST_FILE} missing fn ${fn}" >&2
    exit 1
  fi
done

# ---- 25. baseline JSON exists -------------------------------------------
echo "ASSERT 25: ${BASELINE_JSON} exists"
if [[ ! -f "${BASELINE_JSON}" ]]; then
  echo "FAIL: missing ${BASELINE_JSON}" >&2
  exit 1
fi

# ---- 26. baseline JSON is valid JSON ------------------------------------
echo "ASSERT 26: ${BASELINE_JSON} is valid JSON"
if ! jq -e '.' "${BASELINE_JSON}" >/dev/null 2>&1; then
  echo "FAIL: ${BASELINE_JSON} is not valid JSON" >&2
  exit 1
fi

# ---- 27. baseline JSON has .groups | length == 3 ------------------------
echo "ASSERT 27: ${BASELINE_JSON} has .groups | length == 3"
GROUPS_LEN=$(jq '.groups | length' "${BASELINE_JSON}")
if [[ "${GROUPS_LEN}" != "3" ]]; then
  echo "FAIL: ${BASELINE_JSON} .groups | length expected 3, got ${GROUPS_LEN}" >&2
  exit 1
fi

# ---- 28. every group entry has all required fields ---------------------
echo "ASSERT 28: each group in ${BASELINE_JSON} has name, p50_us, p95_us, p99_us, throughput_eps"
for field in name p50_us p95_us p99_us throughput_eps; do
  MISSING=$(jq -r "[.groups[] | select(has(\"${field}\") | not)] | length" "${BASELINE_JSON}")
  if [[ "${MISSING}" != "0" ]]; then
    echo "FAIL: ${BASELINE_JSON} has ${MISSING} group(s) missing field '${field}'" >&2
    exit 1
  fi
done

# ---- 29. no scope creep into src/main.rs or crates/archon-tui/src/ ------
echo "ASSERT 29: src/main.rs and crates/archon-tui/src/ untouched"
SCOPE_CREEP=$(git diff --name-only src/main.rs crates/archon-tui/src/ 2>/dev/null | wc -l | tr -d ' ')
if [[ "${SCOPE_CREEP}" != "0" ]]; then
  echo "FAIL: src/main.rs or crates/archon-tui/src/ was modified (out of scope for TUI-009)" >&2
  git diff --name-only src/main.rs crates/archon-tui/src/ 2>/dev/null >&2 || true
  exit 1
fi

echo "OK: verify-TUI-009 passed"
