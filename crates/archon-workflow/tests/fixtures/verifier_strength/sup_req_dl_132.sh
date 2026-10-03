set -eu
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
run() {
  lane="$1"
  log="$2"
  shift 2
  if ! cargo test -p archon-trading "$@" > "$log" 2>&1; then
    echo "SUP-REQ-DL-132 false: $lane: the pinned migration test target failed to build or run, so the idempotency deliverable is missing or broken" >&2
    cat "$log" >&2 || true
    exit 1
  fi
}
pin() {
  lane="$1"
  log="$2"
  name="$3"
  if ! grep -Fq "$name ... ok" "$log"; then
    echo "SUP-REQ-DL-132 false: $lane: pinned idempotency test $name did not execute and pass" >&2
    cat "$log" >&2 || true
    exit 1
  fi
  if ! grep -Eq 'test result: ok\. 1 passed; 0 failed' "$log"; then
    echo "SUP-REQ-DL-132 false: $lane: pinned idempotency test $name did not run exactly once (vacuous filter or extra failures)" >&2
    cat "$log" >&2 || true
    exit 1
  fi
}
run 'data-lake migration second-run lane' "$T/dl.log" --test migration_v1_to_v2 second_run_is_byte_identical_no_op
pin 'data-lake migration second-run lane' "$T/dl.log" second_run_is_byte_identical_no_op
run 'hand-authored-v2 no-op lane' "$T/libv2.log" --lib v2_registry_migration_is_a_byte_identical_no_op
pin 'hand-authored-v2 no-op lane' "$T/libv2.log" v2_registry_migration_is_a_byte_identical_no_op
run 'data-store repeated-migration lane' "$T/store.log" --test registry_migration_v1 counts_reconcile_and_second_run_is_byte_idempotent
pin 'data-store repeated-migration lane' "$T/store.log" counts_reconcile_and_second_run_is_byte_idempotent
run 'data-store published-v2 lane' "$T/storev2.log" --lib v2_migration_report_is_idempotent_and_skips_existing_v2
pin 'data-store published-v2 lane' "$T/storev2.log" v2_migration_report_is_idempotent_and_skips_existing_v2
echo 'SUP-REQ-DL-132 verified: re-running the shipped v1->v2 registry migration on an already-migrated registry is a byte-identical no-op (changed=false, zero counts, no new backup) for both a migration-produced and a hand-authored v2 registry, and the data-store migration lane returns equal all-zero reports with an unchanged registry.json across repeated runs on a populated v1-downgraded lake and a published v2 lake'
