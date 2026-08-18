# Task 1 Report: Freeze Shared Plan Contracts

**Task ID:** `TASK-ISSUE-181-PLAN-CONTRACT`
**Status:** Complete
**Date:** 2026-08-14

## Delivered

- Replaced the string plan lifecycle with typed `PlanStatus`, preserving legacy JSON compatibility: legacy `"active"` deserializes as `Executing`; new serialization uses the canonical `"executing"` form.
- Added typed approval, reconciliation, dependency, and plan-step evidence contracts. New persisted fields default safely when absent from legacy plan JSON.
- Added the durable Cozo `plan_approval_events` ledger keyed by `(session_id, plan_id, decided_at)`. Approval insertion uses `:insert`, preventing duplicate-key overwrites.
- Added typed `PlanModeState` and `PlanEntryPath`. Permission restoration is exhaustive and safe: absent state and unavailable bypass access resolve to `PermissionMode::Default`; repeated plan entry preserves the original captured permission mode and path.
- Split the session plan implementation into `plan_models.rs` and `plan_store.rs` to comply with the project file-size limit while retaining the public `archon_session::plan` API.

## Verification

All six required dev-flow gates passed:

1. `01-tests-written-first` — recorded RED test evidence.
2. `02-implementation-complete` — recorded compile/file-size evidence.
3. `03-sherlock-code-review` — APPROVED independent review.
4. `04-tests-passing` — 12 focused tests passed:
   - `cargo test -p archon-session plan::tests --locked`: 8 passed.
   - `cargo test -p archon-core plan_mode_state --locked`: 4 passed.
5. `05-live-smoke-test` — executed successfully:
   ```bash
   cargo test -p archon-core plan_mode_state::tests::missing_previous_mode_restores_default_not_auto --locked -- --exact
   ```
6. `06-sherlock-final-review` — APPROVED cold review; it also verified the two focused test commands and:
   ```bash
   cargo check -p archon-session -p archon-core --all-targets --locked
   ```

Final gate command:

```bash
/home/unixdude/Archon-projects/archon/scripts/dev-flow-gate.sh TASK-ISSUE-181-PLAN-CONTRACT
```

Result: `ALL 6 GATES PASSED`.

## Inspectable Gate Evidence

The following artifacts were read directly from the task worktree:

- `.gates/TASK-ISSUE-181-PLAN-CONTRACT/01-tests-written-first.passed` — `PASSED`; evidence: `RED: legacy plan defaults and safe permission restoration tests failed before implementation`.
- `.gates/TASK-ISSUE-181-PLAN-CONTRACT/02-implementation-complete.passed` — `PASSED`; evidence: typed lifecycle and safe restore compile, focused tests pass, and `FileSizeGuard: exit 0`.
- `.gates/TASK-ISSUE-181-PLAN-CONTRACT/03-sherlock-code-review.passed` — `PASSED`; verdict text: `APPROVED: independent Sherlock review verified serde compatibility, immutable approval ledger, re-entry preservation, and exhaustive safe restore`.
- `.gates/TASK-ISSUE-181-PLAN-CONTRACT/04-tests-passing.passed` — `PASSED`; updated after the compatibility regression fix with the exact focused test count below.
- `.gates/TASK-ISSUE-181-PLAN-CONTRACT/05-live-smoke-test.passed` — `PASSED`; evidence: `EXEC_VERIFIED (exit 0): cargo test -p archon-core plan_mode_state::tests::missing_previous_mode_restores_default_not_auto --locked -- --exact`.
- `.gates/TASK-ISSUE-181-PLAN-CONTRACT/06-sherlock-final-review.passed` — `PASSED`; verdict text: `APPROVED: cold final Sherlock review verified legacy serde compatibility, append-only Cozo ledger, exhaustive safe permission restoration, and re-entry preservation; locked focused tests and check passed`.

### Commands and Output Summaries

Initial regression RED command:

```bash
cargo test -p archon-session legacy_complete_plan_json_loads_with_safe_defaults --locked
```

Before the alias fix this exited `101` with `unknown variant 'complete'`, demonstrating the legacy-deserialization defect.

After adding `#[serde(alias = "complete")]` to `PlanStatus::Completed`, the following commands succeeded:

```bash
cargo test -p archon-session legacy_complete_plan_json_loads_with_safe_defaults --locked
cargo test -p archon-session plan::tests --locked
cargo test -p archon-core plan_mode_state --locked
cargo check -p archon-session -p archon-core --all-targets --locked
```

Results: the targeted compatibility test passed `1`; session plan tests passed `8`; plan-mode-state tests passed `4`; the locked all-target check completed successfully. The expanded legacy tests assert defaults for `approval`, `reconciliation`, `session_id`, `branch`, `commits`, and `user_edited`.

## Key Files

- `crates/archon-session/src/plan_models.rs`
- `crates/archon-session/src/plan_store.rs`
- `crates/archon-session/src/plan.rs`
- `crates/archon-core/src/agent/plan_mode_state.rs`
- `crates/archon-core/src/agent/tool_postprocess_steps.rs`

## Known Limitations

- The ledger's duplicate guard relies on caller-provided RFC 3339 timestamps; separate approval events produced with distinct timestamps are intentionally retained rather than deduplicated by decision content.
- The focused validation proves contract, storage, and restoration behavior; it does not execute a full interactive CLI approval session.
