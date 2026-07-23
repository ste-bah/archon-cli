#!/usr/bin/env bash
set -uo pipefail
TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPTS_DIR="$(cd "$TEST_DIR/.." && pwd)"
CI_GATE_SOURCE="${CI_GATE_SOURCE:-$SCRIPTS_DIR/ci-gate.sh}"
REGEN_SOURCE="${REGEN_SOURCE:-$SCRIPTS_DIR/regen-baseline.sh}"
LIST_SOURCE="${LIST_SOURCE:-$SCRIPTS_DIR/list-cargo-tests.sh}"
PASS_COUNT=0
FAIL_COUNT=0
pass() {
  PASS_COUNT=$((PASS_COUNT + 1))
  printf 'PASS  %s\n' "$1"
}
fail() {
  FAIL_COUNT=$((FAIL_COUNT + 1))
  printf 'FAIL  %s: %s\n' "$1" "$2"
}
make_sandbox() {
  local sandbox
  sandbox="$(mktemp -d)"
  mkdir -p "$sandbox/scripts" "$sandbox/tests/fixtures/baseline" "$sandbox/bin" "$sandbox/test-bin"
  cp "$CI_GATE_SOURCE" "$sandbox/scripts/ci-gate.sh"
  cp "$REGEN_SOURCE" "$sandbox/scripts/regen-baseline.sh"
  cp "$LIST_SOURCE" "$sandbox/scripts/list-cargo-tests.sh"
  cat > "$sandbox/bin/cargo" <<'STUB'
#!/usr/bin/env bash
set -u
if [[ "${1:-}" == "metadata" ]]; then
  if [[ "${CARGO_STUB_MULTIPACKAGE:-0}" == "1" ]]; then
    printf '%s\n' '{"workspace_members":["fixture-crate 0.1.0 (path+file:///fixture)","second-crate 0.1.0 (path+file:///second)"],"packages":[{"id":"fixture-crate 0.1.0 (path+file:///fixture)","name":"fixture-crate","targets":[{"name":"fixture_lib","kind":["lib"],"test":true,"doctest":true}]},{"id":"second-crate 0.1.0 (path+file:///second)","name":"second-crate","targets":[{"name":"second_lib","kind":["lib"],"test":true,"doctest":true}]}]}'
    exit 0
  fi
  if [[ -n "${CARGO_STUB_TARGET_DIR:-}" ]]; then
    targets='[{"name":"fixture_lib","kind":["lib"],"test":true,"doctest":true},{"name":"alpha_target","kind":["test"],"test":true,"doctest":false},{"name":"beta_target","kind":["test"],"test":true,"doctest":false}]'
  else
    targets='[{"name":"fixture_lib","kind":["lib"],"test":true,"doctest":true}]'
  fi
  printf '{"workspace_members":["fixture-crate 0.1.0 (path+file:///fixture)"],"packages":[{"id":"fixture-crate 0.1.0 (path+file:///fixture)","name":"fixture-crate","targets":%s}]}\n' "$targets"
  exit 0
fi
if [[ "$*" == *"--list --format=terse"* ]]; then
  if [[ "${CARGO_STUB_FORBID_CARGO_LIST:-0}" == "1" ]]; then
    printf '%s\n' 'per-target cargo list invocation forbidden' >&2
    exit 99
  fi
fi
if [[ "$*" == *"--no-run"* && "$*" == *"--message-format=json"* ]]; then
  if [[ "${CARGO_STUB_REQUIRE_WORKSPACE:-0}" == "1" && "$*" != *"--workspace"* ]]; then
    printf '%s\n' 'single workspace build was not requested' >&2
    exit 97
  fi
  if [[ "${CARGO_STUB_FORBID_ALL_TARGETS:-0}" == "1" && "$*" == *"--all-targets"* ]]; then
    printf '%s\n' 'non-default Cargo targets were requested' >&2
    exit 98
  fi
  targets=(fixture_lib)
  kinds=(lib)
  executables=(fixture_lib)
  packages=('fixture-crate 0.1.0 (path+file:///fixture)')
  if [[ "${CARGO_STUB_SAME_NAME_KINDS:-0}" == "1" ]]; then
    targets=(shared_target shared_target)
    kinds=(lib bin)
    executables=(shared-lib shared-bin)
    packages=('fixture-crate 0.1.0 (path+file:///fixture)' 'fixture-crate 0.1.0 (path+file:///fixture)')
  elif [[ "${CARGO_STUB_MULTIPACKAGE:-0}" == "1" ]]; then
    targets+=(second_lib)
    kinds+=(lib)
    executables+=(second_lib)
    packages+=('second-crate 0.1.0 (path+file:///second)')
  elif [[ -n "${CARGO_STUB_TARGET_DIR:-}" ]]; then
    targets+=(alpha_target beta_target)
    kinds+=(test test)
    executables+=(alpha_target beta_target)
    packages+=('fixture-crate 0.1.0 (path+file:///fixture)' 'fixture-crate 0.1.0 (path+file:///fixture)')
  fi
  for index in "${!targets[@]}"; do
    target="${targets[$index]}"
    kind="${kinds[$index]}"
    executable="${executables[$index]}"
    package_id="${packages[$index]}"
    printf '{"reason":"compiler-artifact","package_id":"%s","target":{"name":"%s","kind":["%s"],"test":true},"profile":{"test":true},"executable":"%s/test-%s"}\n' \
      "$package_id" "$target" "$kind" "$CARGO_STUB_EXEC_DIR" "$executable"
  done
  exit 0
fi
if [[ "${CARGO_STUB_MODE:-list}" == "timeout-run" && "$*" == *"--test-threads=1"* ]]; then
  exit 124
fi
if [[ "${CARGO_STUB_MODE:-list}" == "fail-run" && "$*" == *"--test-threads=1"* ]]; then
  printf '%s\n' 'synthetic cargo test failure' >&2
  exit 43
fi
printf '%s\n' 'test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s'
STUB
  cat > "$sandbox/test-bin/test-harness" <<'STUB'
#!/usr/bin/env bash
set -u
target="${0##*/test-}"
if [[ "${CARGO_STUB_MODE:-list}" == "timeout-list" ]]; then
  exit 124
fi
if [[ "${CARGO_STUB_MODE:-list}" == "fail-list" ]]; then
  printf '%s\n' 'synthetic cargo discovery failure' >&2
  exit 42
fi
if [[ -n "${CARGO_STUB_TARGET_DIR:-}" ]]; then
  cat "$CARGO_STUB_TARGET_DIR/$target.txt"
else
  cat "$CARGO_STUB_LIST_FILE"
fi
STUB
  chmod +x "$sandbox/test-bin/test-harness"
  ln -s test-harness "$sandbox/test-bin/test-fixture_lib"
  ln -s test-harness "$sandbox/test-bin/test-alpha_target"
  ln -s test-harness "$sandbox/test-bin/test-beta_target"
  ln -s test-harness "$sandbox/test-bin/test-second_lib"
  ln -s test-harness "$sandbox/test-bin/test-shared-lib"
  ln -s test-harness "$sandbox/test-bin/test-shared-bin"
  cat > "$sandbox/bin/cp" <<'STUB'
#!/usr/bin/env bash
set -u
if [[ "${CP_STUB_FAIL_ROLLBACK:-0}" == "1" && "${1:-}" == */cargo_test_list.previous ]]; then
  printf '%s\n' 'synthetic list rollback failure' >&2
  exit 57
fi
PATH="${PATH#*:}" exec cp "$@"
STUB
  cat > "$sandbox/bin/mv" <<'STUB'
#!/usr/bin/env bash
set -u
if [[ "${MV_STUB_FAIL_SUMMARY:-0}" == "1" && "${!#}" == */cargo_test_summary.txt ]]; then
  printf '%s\n' 'synthetic summary publication failure' >&2
  exit 55
fi
if [[ "${MV_STUB_FAIL_LIST:-0}" == "1" && "${!#}" == */cargo_test_list.txt ]]; then
  printf '%s\n' 'synthetic list publication failure' >&2
  exit 56
fi
PATH="${PATH#*:}" exec mv "$@"
STUB
  cat > "$sandbox/bin/timeout" <<'STUB'
#!/usr/bin/env bash
shift
exec "$@"
STUB
  chmod +x "$sandbox/bin/cargo" "$sandbox/bin/cp" "$sandbox/bin/mv" "$sandbox/bin/timeout"
  printf '%s' "$sandbox"
}
run_gate() {
  local sandbox="$1"
  shift
  (
    cd "$sandbox"
    PATH="$sandbox/bin:$PATH" CARGO_STUB_EXEC_DIR="$sandbox/test-bin" "$@" bash scripts/ci-gate.sh --only baseline-diff
  ) 2>&1
}
run_list() {
  local sandbox="$1"
  shift
  (
    cd "$sandbox"
    PATH="$sandbox/bin:$PATH" CARGO_STUB_EXEC_DIR="$sandbox/test-bin" "$@" bash scripts/list-cargo-tests.sh tests/fixtures/baseline/cargo_test_list.txt
  ) 2>&1
}
run_regen() {
  local sandbox="$1"
  shift
  (
    cd "$sandbox"
    PATH="$sandbox/bin:$PATH" CARGO_STUB_EXEC_DIR="$sandbox/test-bin" "$@" bash scripts/regen-baseline.sh
  ) 2>&1
}
test_regen_and_gate_share_normalization() {
  local name="regen and gate share normalization"
  local sandbox output rc
  sandbox="$(make_sandbox)"
  cat > "$sandbox/list.txt" <<'LIST'
alpha::tests::one: test
beta::nested::two: test
crates/fixture/src/lib.rs - fixture (line 4): test
src/lib.rs - root_fixture (line 9): test
LIST
  output="$(run_regen "$sandbox" env CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -ne 0 ]]; then
    fail "$name" "regen rc=$rc output=$output"
    rm -rf "$sandbox"
    return
  fi
  if ! diff -u <(printf '%s\n' 'fixture-crate::lib::fixture_lib::alpha::tests::one' 'fixture-crate::lib::fixture_lib::beta::nested::two') \
      "$sandbox/tests/fixtures/baseline/cargo_test_list.txt" >/dev/null; then
    fail "$name" "regen produced unexpected identities"
    rm -rf "$sandbox"
    return
  fi
  output="$(run_gate "$sandbox" env CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -eq 0 ]]; then
    pass "$name"
  else
    fail "$name" "gate rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_discovery_failure_propagates() {
  local name="cargo discovery failure propagates"
  local sandbox output rc
  sandbox="$(make_sandbox)"
  printf '%s\n' 'fixture-crate::lib::fixture_lib::alpha::tests::one' > "$sandbox/tests/fixtures/baseline/cargo_test_list.txt"
  : > "$sandbox/list.txt"
  output="$(run_gate "$sandbox" env CARGO_STUB_MODE=fail-list CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -ne 0 && "$output" == *"synthetic cargo discovery failure"* ]]; then
    pass "$name"
  else
    fail "$name" "gate failed to preserve cargo diagnostic: rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_discovery_timeout_propagates() {
  local name="cargo discovery timeout propagates"
  local sandbox output rc
  sandbox="$(make_sandbox)"
  printf '%s\n' 'fixture-crate::lib::fixture_lib::alpha::tests::one' > "$sandbox/tests/fixtures/baseline/cargo_test_list.txt"
  : > "$sandbox/list.txt"
  output="$(run_gate "$sandbox" env CARGO_STUB_MODE=timeout-list CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -eq 124 && "$output" == *"discovery timed out"* ]]; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_discovery_avoids_per_target_cargo_startup() {
  local name="discovery avoids per-target cargo startup"
  local sandbox output rc
  sandbox="$(make_sandbox)"
  printf '%s\n' 'alpha::tests::one: test' > "$sandbox/list.txt"
  output="$(run_list "$sandbox" env CARGO_STUB_FORBID_CARGO_LIST=1 CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -eq 0 ]]; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_discovery_uses_default_cargo_test_targets() {
  local name="discovery uses default cargo test targets"
  local sandbox output rc
  sandbox="$(make_sandbox)"
  printf '%s\n' 'alpha::tests::one: test' > "$sandbox/list.txt"
  output="$(run_list "$sandbox" env CARGO_STUB_FORBID_ALL_TARGETS=1 CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -eq 0 ]]; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_discovery_uses_one_workspace_build() {
  local name="discovery uses one workspace build"
  local sandbox output rc
  sandbox="$(make_sandbox)"
  printf '%s\n' 'alpha::tests::one: test' > "$sandbox/list.txt"
  output="$(run_list "$sandbox" env CARGO_STUB_MULTIPACKAGE=1 CARGO_STUB_REQUIRE_WORKSPACE=1 CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -eq 0 ]] && [[ "$(wc -l < "$sandbox/tests/fixtures/baseline/cargo_test_list.txt")" -eq 2 ]]; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_failed_discovery_preserves_existing_output() {
  local name="failed discovery preserves existing output"
  local sandbox output rc expected
  sandbox="$(make_sandbox)"
  expected='fixture-crate::lib::fixture_lib::protected::test'
  printf '%s\n' "$expected" > "$sandbox/tests/fixtures/baseline/cargo_test_list.txt"
  : > "$sandbox/list.txt"
  output="$(run_list "$sandbox" env CARGO_STUB_MODE=fail-list CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -ne 0 ]] && [[ "$(cat "$sandbox/tests/fixtures/baseline/cargo_test_list.txt")" == "$expected" ]]; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_genuine_deletion_fails_with_full_identity() {
  local name="genuine deletion reports full identity"
  local sandbox output rc
  sandbox="$(make_sandbox)"
  printf '%s\n' 'fixture-crate::lib::fixture_lib::alpha::tests::one' 'fixture-crate::lib::fixture_lib::beta::tests::removed' \
    > "$sandbox/tests/fixtures/baseline/cargo_test_list.txt"
  printf '%s\n' 'alpha::tests::one: test' > "$sandbox/list.txt"
  output="$(run_gate "$sandbox" env CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -ne 0 && "$output" == *"fixture-crate::lib::fixture_lib::beta::tests::removed"* ]]; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_additions_are_allowed() {
  local name="new tests remain allowed"
  local sandbox output rc
  sandbox="$(make_sandbox)"
  printf '%s\n' 'fixture-crate::lib::fixture_lib::alpha::tests::one' > "$sandbox/tests/fixtures/baseline/cargo_test_list.txt"
  printf '%s\n' 'alpha::tests::one: test' 'beta::tests::new: test' > "$sandbox/list.txt"
  output="$(run_gate "$sandbox" env CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -eq 0 ]]; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_empty_discovery_fails_loudly() {
  local name="empty discovery fails loudly"
  local sandbox output rc
  sandbox="$(make_sandbox)"
  : > "$sandbox/tests/fixtures/baseline/cargo_test_list.txt"
  : > "$sandbox/list.txt"
  output="$(run_gate "$sandbox" env CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -ne 0 && "$output" == *"no unit tests discovered"* ]]; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_empty_regeneration_fails_loudly() {
  local name="empty regeneration fails loudly"
  local sandbox output rc
  sandbox="$(make_sandbox)"
  : > "$sandbox/list.txt"
  output="$(run_regen "$sandbox" env CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -ne 0 && "$output" == *"no unit tests discovered"* ]]; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_regeneration_test_failure_propagates() {
  local name="regeneration test failure propagates"
  local sandbox output rc
  sandbox="$(make_sandbox)"
  printf '%s\n' 'alpha::tests::one: test' > "$sandbox/list.txt"
  output="$(run_regen "$sandbox" env CARGO_STUB_MODE=fail-run CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -ne 0 && "$output" == *"test run failed (exit 43)"* ]]; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_regeneration_timeout_fails_loudly() {
  local name="regeneration timeout fails loudly"
  local sandbox output rc
  sandbox="$(make_sandbox)"
  printf '%s\n' 'alpha::tests::one: test' > "$sandbox/list.txt"
  output="$(run_regen "$sandbox" env CARGO_STUB_MODE=timeout-run CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -eq 124 && "$output" == *"TIMEOUT"* ]]; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_list_publication_failure_propagates() {
  local name="list publication failure propagates"
  local sandbox output rc expected
  sandbox="$(make_sandbox)"
  expected='fixture-crate::lib::fixture_lib::protected::test'
  printf '%s\n' "$expected" > "$sandbox/tests/fixtures/baseline/cargo_test_list.txt"
  printf '%s\n' 'alpha::tests::one: test' > "$sandbox/list.txt"
  output="$(run_list "$sandbox" env MV_STUB_FAIL_LIST=1 CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -eq 56 && "$output" == *"synthetic list publication failure"* ]] && \
      [[ "$(cat "$sandbox/tests/fixtures/baseline/cargo_test_list.txt")" == "$expected" ]]; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_summary_rollback_failure_propagates() {
  local name="summary rollback failure propagates"
  local sandbox output rc
  sandbox="$(make_sandbox)"
  printf '%s\n' 'fixture-crate::lib::fixture_lib::protected::test' > "$sandbox/tests/fixtures/baseline/cargo_test_list.txt"
  printf '%s\n' 'alpha::tests::one: test' > "$sandbox/list.txt"
  output="$(run_regen "$sandbox" env MV_STUB_FAIL_SUMMARY=1 CP_STUB_FAIL_ROLLBACK=1 CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -eq 57 && "$output" == *"synthetic list rollback failure"* ]]; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_summary_publication_failure_propagates() {
  local name="summary publication failure preserves baseline pair"
  local sandbox output rc expected_list expected_summary
  sandbox="$(make_sandbox)"
  expected_list='fixture-crate::lib::fixture_lib::protected::test'
  expected_summary='passed=7 failed=0 ignored=1'
  printf '%s\n' "$expected_list" > "$sandbox/tests/fixtures/baseline/cargo_test_list.txt"
  printf '%s\n' "$expected_summary" > "$sandbox/tests/fixtures/baseline/cargo_test_summary.txt"
  printf '%s\n' 'alpha::tests::one: test' > "$sandbox/list.txt"
  output="$(run_regen "$sandbox" env MV_STUB_FAIL_SUMMARY=1 CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -eq 55 && "$output" == *"synthetic summary publication failure"* ]] && \
      [[ "$(cat "$sandbox/tests/fixtures/baseline/cargo_test_list.txt")" == "$expected_list" ]] && \
      [[ "$(cat "$sandbox/tests/fixtures/baseline/cargo_test_summary.txt")" == "$expected_summary" ]]; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_same_named_targets_of_different_kinds_remain_distinct() {
  local name="same-named different-kind targets remain distinct"
  local sandbox output rc
  sandbox="$(make_sandbox)"
  printf '%s\n' 'shared::works: test' > "$sandbox/list.txt"
  output="$(run_list "$sandbox" env CARGO_STUB_SAME_NAME_KINDS=1 CARGO_STUB_LIST_FILE="$sandbox/list.txt")"
  rc=$?
  if [[ "$rc" -eq 0 ]] && diff -u \
      <(printf '%s\n' \
        'fixture-crate::bin::shared_target::shared::works' \
        'fixture-crate::lib::shared_target::shared::works') \
      "$sandbox/tests/fixtures/baseline/cargo_test_list.txt" >/dev/null; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output identities=$(cat "$sandbox/tests/fixtures/baseline/cargo_test_list.txt")"
  fi
  rm -rf "$sandbox"
}
test_same_named_tests_in_different_targets_do_not_mask_deletion() {
  local name="same-named cross-target tests do not mask deletion"
  local sandbox output rc
  sandbox="$(make_sandbox)"
  printf '%s\n' \
    'fixture-crate::test::alpha_target::shared::works' \
    'fixture-crate::test::beta_target::shared::works' \
    > "$sandbox/tests/fixtures/baseline/cargo_test_list.txt"
  mkdir -p "$sandbox/targets"
  : > "$sandbox/targets/alpha_target.txt"
  printf '%s\n' 'shared::works: test' > "$sandbox/targets/beta_target.txt"
  : > "$sandbox/targets/fixture_lib.txt"
  output="$(run_gate "$sandbox" env CARGO_STUB_TARGET_DIR="$sandbox/targets")"
  rc=$?
  if [[ "$rc" -ne 0 && "$output" == *"fixture-crate::test::alpha_target::shared::works"* ]]; then
    pass "$name"
  else
    fail "$name" "rc=$rc output=$output"
  fi
  rm -rf "$sandbox"
}
test_regen_and_gate_share_normalization
test_discovery_failure_propagates
test_discovery_timeout_propagates
test_discovery_avoids_per_target_cargo_startup
test_discovery_uses_default_cargo_test_targets
test_discovery_uses_one_workspace_build
test_failed_discovery_preserves_existing_output
test_empty_discovery_fails_loudly
test_empty_regeneration_fails_loudly
test_regeneration_test_failure_propagates
test_regeneration_timeout_fails_loudly
test_list_publication_failure_propagates
test_summary_rollback_failure_propagates
test_summary_publication_failure_propagates
test_same_named_targets_of_different_kinds_remain_distinct
test_same_named_tests_in_different_targets_do_not_mask_deletion
test_genuine_deletion_fails_with_full_identity
test_additions_are_allowed
printf '\n%d passed; %d failed\n' "$PASS_COUNT" "$FAIL_COUNT"
[[ "$FAIL_COUNT" -eq 0 ]]
