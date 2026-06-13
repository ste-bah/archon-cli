#!/usr/bin/env bash
# Focused workflow/write-coordinator test harness.
#
# Why this exists:
# - `cargo test -p archon-workflow some_filter` still launches every integration
#   test binary and filters inside each binary, creating long quiet runs.
# - On macOS, external-volume target binaries can be inspected by endpoint
#   security/notarization tooling, which makes test launches look wedged.
#
# This harness runs exact test targets, emits heartbeats, and kills a command
# that produces no completion before the configured timeout.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

HEARTBEAT_SECONDS="${ARCHON_TEST_HEARTBEAT_SECONDS:-20}"
TIMEOUT_SECONDS="${ARCHON_TEST_TIMEOUT_SECONDS:-900}"
LOG_TAIL_LINES="${ARCHON_TEST_LOG_TAIL_LINES:-12}"

export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${TMPDIR:-/tmp}/archon-workflow-focused-target}"

mkdir -p "$CARGO_TARGET_DIR"

terminate_tree() {
  local pid="$1"
  pkill -TERM -P "$pid" 2>/dev/null || true
  kill -TERM "$pid" 2>/dev/null || true
  sleep 2
  pkill -KILL -P "$pid" 2>/dev/null || true
  kill -KILL "$pid" 2>/dev/null || true
}

run_step() {
  local name="$1"
  shift
  local log
  log="$(mktemp "${TMPDIR:-/tmp}/archon-workflow-test.XXXXXX.log")"
  echo
  echo "==> $name"
  echo "+ $*"
  echo "target: $CARGO_TARGET_DIR"

  set +e
  "$@" >"$log" 2>&1 &
  local pid="$!"
  local started
  started="$(date +%s)"
  local status=0

  while kill -0 "$pid" 2>/dev/null; do
    sleep "$HEARTBEAT_SECONDS"
    kill -0 "$pid" 2>/dev/null || break
    local elapsed=$(( $(date +%s) - started ))
    if (( elapsed >= TIMEOUT_SECONDS )); then
      echo "::error::$name timed out after ${elapsed}s; terminating pid $pid"
      tail -n "$LOG_TAIL_LINES" "$log" | sed 's/^/[last] /'
      terminate_tree "$pid"
      wait "$pid" >/dev/null 2>&1
      cat "$log"
      rm -f "$log"
      exit 124
    fi
    echo "[$name] still running after ${elapsed}s (pid $pid)"
    tail -n "$LOG_TAIL_LINES" "$log" | sed 's/^/[last] /'
  done

  wait "$pid"
  status="$?"
  set -e
  cat "$log"
  rm -f "$log"
  if (( status != 0 )); then
    echo "::error::$name failed with exit $status"
    exit "$status"
  fi
}

run_step "coordinated implementation fanout" \
  cargo test -p archon-workflow --test coordinated_implementation_fanout -- --nocapture

run_step "write coordinator config" \
  cargo test -p archon-workflow --test write_coordinator_config -- --nocapture

run_step "coordinated fanout integration" \
  cargo test -p archon-workflow --test coordinated_fanout -- --nocapture

run_step "patch apply unit tests" \
  cargo test -p archon-workflow --lib write_coordinator::patch_apply -- --nocapture

run_step "patch manifest unit tests" \
  cargo test -p archon-workflow --lib write_coordinator::patch_manifest -- --nocapture

if [[ "${ARCHON_WORKFLOW_CHECK_BIN:-0}" == "1" ]]; then
  run_step "archon binary check" cargo check --bin archon
fi

echo
echo "workflow focused tests: ok"
