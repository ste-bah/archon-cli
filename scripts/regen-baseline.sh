#!/usr/bin/env bash
# regen-baseline.sh — Capture a deterministic cargo test baseline snapshot.
#
# SPEC DIVERGENCE (approved 2026-04-11):
#   1. Spec §"Scope — In Scope" mandates a single `cargo test --workspace
#      ... -- --list --format=terse` invocation. Reality: the archon-cli
#      workspace has crashed WSL2 twice under `cargo test --workspace`
#      (unbounded rustc + test parallelism). This script therefore loops
#      per-crate with `--jobs 1 --test-threads=1`. Same output domain,
#      safe under WSL2.
#   2. Doctest rows use source-path descriptions rather than libtest names.
#      They are excluded from the unit/integration-test identity fixture.
#
# SAFETY: This workspace has crashed WSL2 when cargo runs unconstrained.
# Executable test runs use --jobs 1 and --test-threads=1. Test discovery
# uses --jobs 1 and enumerates targets sequentially. Do NOT parallelize.
#
# Outputs (all deterministic):
#   tests/fixtures/baseline/cargo_test_list.txt     — sorted unique package::kind::target::test identities
#   tests/fixtures/baseline/cargo_test_summary.txt  — one line: passed=N failed=M ignored=K
#
# Usage: bash scripts/regen-baseline.sh

set -uo pipefail

# Resolve repo root (script lives in scripts/)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

BASELINE_DIR="tests/fixtures/baseline"
mkdir -p "$BASELINE_DIR"

LIST_FILE="$BASELINE_DIR/cargo_test_list.txt"
SUMMARY_FILE="$BASELINE_DIR/cargo_test_summary.txt"
LIST_STAGE="$BASELINE_DIR/.cargo-test-list.stage"

TMPDIR_RUN="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_RUN"; rm -f "$LIST_STAGE"' EXIT

if ! METADATA_JSON="$(cargo metadata --no-deps --format-version 1)"; then
  echo "[regen-baseline] cargo metadata failed" >&2
  exit 1
fi
if ! CRATE_LIST="$(python3 -c 'import json, sys; data=json.load(sys.stdin); members=set(data["workspace_members"]); print("\n".join(sorted(p["name"] for p in data["packages"] if p["id"] in members)))' <<< "$METADATA_JSON")"; then
  echo "[regen-baseline] workspace member parsing failed" >&2
  exit 1
fi
mapfile -t CRATES <<< "$CRATE_LIST"
if [[ "${#CRATES[@]}" -eq 0 || -z "${CRATES[0]}" ]]; then
  echo "[regen-baseline] workspace contains no packages" >&2
  exit 1
fi

TOTAL_PASSED=0
TOTAL_FAILED=0
TOTAL_IGNORED=0
for crate in "${CRATES[@]}"; do
  RUN_LOG="$TMPDIR_RUN/run-$crate.log"
  timeout 600 cargo test -p "$crate" --no-fail-fast --jobs 1 -- --test-threads=1 \
    > "$RUN_LOG" 2>&1
  rc=$?
  if [[ "$rc" -eq 124 ]]; then
    echo "[regen-baseline] $crate: TIMEOUT" >&2
    exit "$rc"
  elif [[ "$rc" -ne 0 ]]; then
    echo "[regen-baseline] $crate: test run failed (exit $rc)" >&2
    exit "$rc"
  fi
  while IFS= read -r line; do
    passed=$(sed -nr 's/.*test result:[^0-9]*([0-9]+) passed.*/\1/p' <<< "$line")
    failed=$(sed -nr 's/.*test result:[^;]*;[^0-9]*([0-9]+) failed.*/\1/p' <<< "$line")
    ignored=$(sed -nr 's/.*test result:[^;]*;[^;]*;[^0-9]*([0-9]+) ignored.*/\1/p' <<< "$line")
    [[ -n "$passed" ]] && TOTAL_PASSED=$((TOTAL_PASSED + passed))
    [[ -n "$failed" ]] && TOTAL_FAILED=$((TOTAL_FAILED + failed))
    [[ -n "$ignored" ]] && TOTAL_IGNORED=$((TOTAL_IGNORED + ignored))
  done < <(grep -E '^test result:' "$RUN_LOG" || true)
done

if bash scripts/list-cargo-tests.sh "$LIST_STAGE"; then
  :
else
  discovery_rc=$?
  echo "[regen-baseline] test discovery failed" >&2
  exit "$discovery_rc"
fi

if SUMMARY_TMP="$(mktemp "$BASELINE_DIR/.cargo-test-summary.XXXXXX")"; then
  :
else
  summary_rc=$?
  echo "[regen-baseline] summary temporary file creation failed" >&2
  exit "$summary_rc"
fi
if printf 'passed=%d failed=%d ignored=%d\n' \
    "$TOTAL_PASSED" "$TOTAL_FAILED" "$TOTAL_IGNORED" > "$SUMMARY_TMP"; then
  :
else
  summary_rc=$?
  rm -f "$SUMMARY_TMP"
  echo "[regen-baseline] summary write failed" >&2
  exit "$summary_rc"
fi
LIST_BACKUP="$TMPDIR_RUN/cargo_test_list.previous"
list_existed=0
if [[ -e "$LIST_FILE" ]]; then
  cp "$LIST_FILE" "$LIST_BACKUP" || exit $?
  list_existed=1
fi
if mv "$LIST_STAGE" "$LIST_FILE"; then
  :
else
  list_rc=$?
  echo "[regen-baseline] list publication failed" >&2
  exit "$list_rc"
fi
if mv "$SUMMARY_TMP" "$SUMMARY_FILE"; then
  :
else
  summary_rc=$?
  rm -f "$SUMMARY_TMP"
  if [[ "$list_existed" -eq 1 ]]; then
    if cp "$LIST_BACKUP" "$LIST_FILE"; then
      :
    else
      rollback_rc=$?
      echo "[regen-baseline] list rollback failed" >&2
      exit "$rollback_rc"
    fi
  elif rm -f "$LIST_FILE"; then
    :
  else
    rollback_rc=$?
    echo "[regen-baseline] list rollback failed" >&2
    exit "$rollback_rc"
  fi
  echo "[regen-baseline] summary publication failed" >&2
  exit "$summary_rc"
fi

echo "[regen-baseline] Done." >&2
echo "[regen-baseline] list:    $(wc -l < "$LIST_FILE") lines" >&2
echo "[regen-baseline] summary: $(cat "$SUMMARY_FILE")" >&2
