# Workflow Runtime Rescue — Phase 4 Report: Error-as-Value

Branch: `rescue/workflow-runtime-v3`
Result: the terminal latch is gone; task-level failures are structured result
values the script consumes; script-owned remediation branches are reachable.
The canary stays green.

## Semantics changes

- `terminal_stop_for_call`: only cancellation and unsatisfied FinalReport /
  HumanGate calls unwind a script. Failed agents, reduces, fanouts, quality
  gates, and requireArtifact checks return `{status: "failed"|"needs_review",
  ...}` values.
- Deleted `terminal_latched` + `ensure_generated_prd_not_terminally_latched`
  (the latch that blocked all host calls after a terminal report). The runaway
  guards remain `maxRepairIterations`/`maxInvestigationIterations` and the
  60s JS watchdog, per the handover.
- Summary status: a FinalReport's status is authoritative for the run summary,
  so a script-recovered failure no longer dooms an otherwise accepted run;
  runs that fail without reaching a final report keep the failure status.

## Deleted

- `crates/archon-workflow/src/v2/remediation.rs` (349 lines) — consumed only
  by its own test; the lifecycle's remediation logic superseded it.
- `crates/archon-workflow/src/quality_gate.rs` (70 lines) — zero consumers;
  the one host-side qualityGate implementation is the completion-ledger gate
  behind `w.qualityGate` (`final_acceptance_gate_result`), as prescribed.
- `tests/v2_remediation.rs`.

## Tests updated to the new contract

- `failed_reduce_returns_error_value_for_script_owned_remediation` (was
  `..._rejects_await_and_stops...`): the failed reduce flows to the script and
  the remediation branch executes.
- `non_accepted_quality_gate_returns_value_the_script_consumes` (was
  `..._stops_before_final_report`): the script reports the gate outcome.
- `non_accepted_final_report_still_ends_the_script` (replaces the latch test
  `generated_prd_terminal_stop_cannot_be_caught_and_bypassed`, whose premise
  was the deleted latch).

## Approved-pattern deviation (same shape as Phase 2's Option B)

`remediation_inventory.rs` / `remediation_items.rs` / `remediation_noop.rs`
are woven exclusively into the legacy executor (`executor_live`,
`executor_fanout*`), which Phase 5 deletes wholesale; deleting them now would
mean stubbing call sites in next-phase-dead code. They move to Phase 5 with
their consumers. The Phase 4 essence — errors as values, latch gone,
script-owned remediation reachable — is complete.

## Status

- Canary green; 289 workflow bin tests green; 1 pre-existing bin red
  (read-only tools heuristic, Phase 5) and 5 pre-existing crate reds (legacy
  `generated_task_items` ×2, `runtime_continuation` ×3, Phase 5).
- `cargo clippy -p archon-workflow`: clean. FileSizeGuard: green.

## Definition-of-done greps

`terminal_latched`: zero references. `start_with_harness` remains only on the
legacy spec-execution path (Phase 5). `harness_lex|harness_parse_call|
harness_collect`: zero since Phase 1.
